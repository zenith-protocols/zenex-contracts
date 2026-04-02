use crate::constants::{ONE_HOUR_SECONDS, SCALAR_7, SCALAR_18};
use crate::types::MarketData;
use crate::trading::rates;
use soroban_fixed_point_math::SorobanFixedPoint;
use soroban_sdk::Env;

impl Default for MarketData {
    fn default() -> Self {
        Self {
            l_notional: 0,
            s_notional: 0,
            l_fund_idx: 0,
            s_fund_idx: 0,
            l_borr_idx: 0,
            s_borr_idx: 0,
            l_entry_wt: 0,
            s_entry_wt: 0,
            fund_rate: 0,
            last_update: 0,
            l_adl_idx: SCALAR_18,
            s_adl_idx: SCALAR_18,
        }
    }
}

/// Compute utilization = notional / (vault_balance × max_util / SCALAR_7), clamped to [0, SCALAR_7].
fn calc_util(e: &Env, notional: i128, vault_balance: i128, max_util: i128) -> i128 {
    if vault_balance <= 0 || notional <= 0 || max_util <= 0 {
        return 0;
    }
    let cap = vault_balance.fixed_mul_floor(e, &max_util, &SCALAR_7);
    if cap <= 0 { return 0; }
    notional.fixed_div_ceil(e, &cap, &SCALAR_7).min(SCALAR_7)
}

impl MarketData {
    /// Returns (funding_index, borrowing_index, adl_index) for the given side.
    pub fn indices(&self, is_long: bool) -> (i128, i128, i128) {
        if is_long {
            (self.l_fund_idx, self.l_borr_idx, self.l_adl_idx)
        } else {
            (self.s_fund_idx, self.s_borr_idx, self.s_adl_idx)
        }
    }

    /// Returns true if the given side is dominant (has more notional).
    /// `extra` is additional notional being added/removed.
    pub fn is_dominant(&self, is_long: bool, extra: i128) -> bool {
        if is_long {
            self.l_notional + extra > self.s_notional
        } else {
            self.s_notional + extra > self.l_notional
        }
    }

    /// Accrue borrowing then funding indices to the current ledger timestamp.
    ///
    /// Computes vault and market utilization internally from the raw inputs,
    /// then delegates to `calc_borrowing_rate` with the normalized values.
    #[allow(clippy::too_many_arguments)]
    pub fn accrue(
        &mut self,
        e: &Env,
        r_base: i128,
        r_var: i128,
        r_var_market: i128,
        vault_balance: i128,
        total_notional: i128,
        max_util: i128,
        max_util_market: i128,
    ) {
        // No positions → no fees to charge
        if self.l_notional == 0 && self.s_notional == 0 {
            return;
        }

        let current_time = e.ledger().timestamp();
        let seconds = current_time.saturating_sub(self.last_update) as i128;
        self.last_update = current_time;

        if seconds == 0 {
            return;
        }

        let hour = ONE_HOUR_SECONDS as i128;

        // Compute normalized utilizations [0, SCALAR_7]
        let market_notional = self.l_notional + self.s_notional;
        let util_vault = calc_util(e, total_notional, vault_balance, max_util);
        let util_market = calc_util(e, market_notional, vault_balance, max_util_market);

        let borr_rate = rates::calc_borrowing_rate(e, r_base, r_var, r_var_market, util_vault, util_market);

        if borr_rate > 0 {
            let borrow_delta = borr_rate.fixed_mul_ceil(e, &seconds, &hour);
            if self.l_notional > self.s_notional {
                self.l_borr_idx += borrow_delta;
            } else if self.s_notional > self.l_notional {
                self.s_borr_idx += borrow_delta;
            } else if self.l_notional > 0 {
                self.l_borr_idx += borrow_delta;
                self.s_borr_idx += borrow_delta;
            }
        }

        // Funding is peer-to-peer — skip if either side is empty.
        if self.fund_rate == 0 || self.l_notional == 0 || self.s_notional == 0 {
            return;
        }

        let pay_delta = self.fund_rate.abs().fixed_mul_ceil(e, &seconds, &hour);

        let (pay_notional, recv_notional) = if self.fund_rate > 0 {
            (self.l_notional, self.s_notional)
        } else {
            (self.s_notional, self.l_notional)
        };

        // recv_delta scaled by pay/recv ratio so total paid = total received.
        let recv_delta = if recv_notional > 0 {
            let ratio = pay_notional.fixed_div_floor(e, &recv_notional, &SCALAR_18);
            pay_delta.fixed_mul_floor(e, &ratio, &SCALAR_18)
        } else {
            0
        };

        if self.fund_rate > 0 {
            self.l_fund_idx += pay_delta;
            self.s_fund_idx -= recv_delta;
        } else {
            self.s_fund_idx += pay_delta;
            self.l_fund_idx -= recv_delta;
        }
    }

    pub fn update_funding_rate(&mut self, e: &Env, base_funding_rate: i128) {
        self.fund_rate = rates::calc_funding_rate(
            e,
            self.l_notional,
            self.s_notional,
            base_funding_rate,
        );
    }

    /// Updates open interest and entry-weighted aggregate stats.
    /// notional_size: positive for open, negative for close/reduce.
    /// ew_delta: pre-computed |notional| / entry_price in price_scalar precision.
    pub fn update_stats(&mut self, is_long: bool, notional_size: i128, ew_delta: i128) {
        if is_long {
            self.l_notional += notional_size;
            if notional_size > 0 {
                self.l_entry_wt += ew_delta;
            } else {
                self.l_entry_wt -= ew_delta;
            }
        } else {
            self.s_notional += notional_size;
            if notional_size > 0 {
                self.s_entry_wt += ew_delta;
            } else {
                self.s_entry_wt -= ew_delta;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::constants::{SCALAR_7, SCALAR_18};
    use crate::testutils::{create_trading, default_market_data, jump};
    use soroban_sdk::Env;

    const BASE_RATE: i128 = 10_000_000_000_000;
    const VAULT: i128 = 100_000 * SCALAR_7;
    const MAX_UTIL: i128 = 10 * SCALAR_7;
    const MAX_UTIL_MKT: i128 = 5 * SCALAR_7;

    #[test]
    fn test_market_data_update_stats() {
        let e = Env::default();
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            use soroban_fixed_point_math::SorobanFixedPoint;

            let scalar_7: i128 = 10_000_000;
            let price_scalar = scalar_7;
            let entry_price: i128 = 100_000 * scalar_7;

            let mut data = default_market_data();

            let notional_long = 10_000 * scalar_7;
            let notional_short = 5_000 * scalar_7;

            let ew = notional_long.fixed_div_floor(&e, &entry_price, &price_scalar);
            data.update_stats(true, notional_long, ew);
            assert_eq!(data.l_notional, notional_long);
            assert_eq!(data.s_notional, 0);
            assert!(data.l_entry_wt > 0);

            let ew = notional_short.fixed_div_floor(&e, &entry_price, &price_scalar);
            data.update_stats(false, notional_short, ew);
            assert_eq!(data.l_notional, notional_long);
            assert_eq!(data.s_notional, notional_short);
            assert!(data.s_entry_wt > 0);

            let ew = notional_short.fixed_div_floor(&e, &entry_price, &price_scalar);
            data.update_stats(true, -notional_short, ew);
            assert_eq!(data.l_notional, notional_long - notional_short);
        });
    }

    #[test]
    fn test_accrue_funding_longs_pay() {
        let e = Env::default();
        jump(&e, 0);
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let mut data = default_market_data();
            data.l_notional = 2000 * SCALAR_18;
            data.s_notional = 1000 * SCALAR_18;
            data.fund_rate = 10_000_000_000_000;
            data.last_update = 0;

            jump(&e, 3600);
            data.accrue(&e, 0, 0, 0, 0, 0, MAX_UTIL, MAX_UTIL_MKT);

            // pay_delta = fund_rate × 3600/3600 = 10_000_000_000_000
            // ratio = floor(L/S) = floor(2000/1000 × S18) = 2 × S18
            // recv_delta = floor(pay_delta × 2 × S18 / S18) = 20_000_000_000_000
            // Shorts receive 2x per-unit (half the notional absorbs the full payment)
            assert_eq!(data.l_fund_idx, 10_000_000_000_000);
            assert_eq!(data.s_fund_idx, -20_000_000_000_000);
            assert_eq!(data.last_update, 3600);
        });
    }

    #[test]
    fn test_accrue_borrowing_longs_dominant() {
        let e = Env::default();
        jump(&e, 0);
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let mut data = default_market_data();
            data.l_notional = 2000 * SCALAR_7;
            data.s_notional = 1000 * SCALAR_7;
            data.last_update = 0;

            jump(&e, 3600);
            let total = data.l_notional + data.s_notional;
            data.accrue(&e, BASE_RATE, 0, 0, VAULT, total, MAX_UTIL, MAX_UTIL_MKT);

            // r_var=0, r_var_market=0 → borr_rate = r_base = BASE_RATE
            // borrow_delta = BASE_RATE × 3600/3600 = 10_000_000_000_000
            assert_eq!(data.l_borr_idx, 10_000_000_000_000, "dominant longs should accrue");
            assert_eq!(data.s_borr_idx, 0, "non-dominant shorts should NOT accrue");
        });
    }

    #[test]
    fn test_accrue_borrowing_shorts_dominant() {
        let e = Env::default();
        jump(&e, 0);
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let mut data = default_market_data();
            data.l_notional = 1000 * SCALAR_7;
            data.s_notional = 2000 * SCALAR_7;
            data.last_update = 0;

            jump(&e, 3600);
            let total = data.l_notional + data.s_notional;
            data.accrue(&e, BASE_RATE, 0, 0, VAULT, total, MAX_UTIL, MAX_UTIL_MKT);

            assert_eq!(data.l_borr_idx, 0, "non-dominant longs should NOT accrue");
            assert_eq!(data.s_borr_idx, 10_000_000_000_000, "dominant shorts should accrue");
        });
    }

    #[test]
    fn test_accrue_borrowing_balanced_both_pay() {
        let e = Env::default();
        jump(&e, 0);
        let (address, _) = create_trading(&e);

        e.as_contract(&address, || {
            let mut data = default_market_data();
            data.l_notional = 1000 * SCALAR_7;
            data.s_notional = 1000 * SCALAR_7;
            data.last_update = 0;

            jump(&e, 3600);
            let total = data.l_notional + data.s_notional;
            data.accrue(&e, BASE_RATE, 0, 0, VAULT, total, MAX_UTIL, MAX_UTIL_MKT);

            // Balanced: both sides pay identical borrowing
            assert_eq!(data.l_borr_idx, 10_000_000_000_000);
            assert_eq!(data.s_borr_idx, 10_000_000_000_000);
        });
    }
}
