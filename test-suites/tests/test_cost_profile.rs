extern crate std;

use trading::constants::SCALAR_7;
use trading::testutils::*;
use trading::PriceData;
use trading::{ExecuteRequest, ExecuteRequestType};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Env};

fn price_data(e: &Env) -> PriceData {
    PriceData {
        feed_id: BTC_FEED_ID,
        price: BTC_PRICE,
        exponent: -8,
        publish_time: e.ledger().timestamp(),
    }
}

fn price_data_at(e: &Env, price: i128) -> PriceData {
    PriceData {
        feed_id: BTC_FEED_ID,
        price,
        exponent: -8,
        publish_time: e.ledger().timestamp(),
    }
}

fn measure<F: FnOnce()>(e: &Env, label: &str, f: F) {
    let mut budget = e.cost_estimate().budget();
    budget.reset_default();
    budget.reset_unlimited();
    f();
    let budget = e.cost_estimate().budget();
    let cpu = budget.cpu_instruction_cost();
    let mem = budget.memory_bytes_cost();
    std::println!("\n=== {} ===", label);
    std::println!("  CPU instructions:   {}", cpu);
    std::println!("  Memory bytes:       {}", mem);
    budget.print();
}

#[test]
fn profile_place_limit() {
    let e = setup_env();
    e.cost_estimate().budget().reset_unlimited();
    let (contract, token_client) = setup_contract(&e);
    let user = Address::generate(&e);
    token_client.mint(&user, &(10_000_000 * SCALAR_7));

    measure(&e, "PLACE LIMIT", || {
        e.as_contract(&contract, || {
            trading::trading::execute_create_limit(
                &e, &user, BTC_FEED_ID, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, BTC_PRICE, 0, 0,
            );
        });
    });
}

#[test]
fn profile_open_market() {
    let e = setup_env();
    e.cost_estimate().budget().reset_unlimited();
    let (contract, token_client) = setup_contract(&e);
    let user = Address::generate(&e);
    token_client.mint(&user, &(10_000_000 * SCALAR_7));

    measure(&e, "OPEN MARKET", || {
        e.as_contract(&contract, || {
            trading::trading::execute_create_market(
                &e, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &price_data(&e),
            );
        });
    });
}

#[test]
fn profile_fill_limit() {
    let e = setup_env();
    e.cost_estimate().budget().reset_unlimited();
    let (contract, token_client) = setup_contract(&e);
    let user = Address::generate(&e);
    let caller = Address::generate(&e);
    token_client.mint(&user, &(10_000_000 * SCALAR_7));

    let id = e.as_contract(&contract, || {
        trading::trading::execute_create_limit(
            &e, &user, BTC_FEED_ID, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, BTC_PRICE, 0, 0,
        )
    });

    measure(&e, "FILL LIMIT", || {
        e.as_contract(&contract, || {
            let requests = vec![&e, ExecuteRequest {
                request_type: ExecuteRequestType::Fill as u32,
                position_id: id,
            }];
            trading::trading::execute_trigger(&e, &caller, requests, &price_data(&e));
        });
    });
}

#[test]
fn profile_close_position() {
    let e = setup_env();
    e.cost_estimate().budget().reset_unlimited();
    let (contract, token_client) = setup_contract(&e);
    let user = Address::generate(&e);
    token_client.mint(&user, &(10_000_000 * SCALAR_7));

    let id = e.as_contract(&contract, || {
        trading::trading::execute_create_market(
            &e, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &price_data(&e),
        )
    });

    jump(&e, 1000 + 31);

    measure(&e, "CLOSE POSITION", || {
        e.as_contract(&contract, || {
            trading::trading::execute_close_position(&e, id, &price_data(&e));
        });
    });
}

#[test]
fn profile_cancel_limit() {
    let e = setup_env();
    e.cost_estimate().budget().reset_unlimited();
    let (contract, token_client) = setup_contract(&e);
    let user = Address::generate(&e);
    token_client.mint(&user, &(10_000_000 * SCALAR_7));

    let id = e.as_contract(&contract, || {
        trading::trading::execute_create_limit(
            &e, &user, BTC_FEED_ID, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, BTC_PRICE, 0, 0,
        )
    });

    measure(&e, "CANCEL LIMIT", || {
        e.as_contract(&contract, || {
            trading::trading::execute_cancel_limit(&e, id);
        });
    });
}

#[test]
fn profile_apply_funding() {
    let e = setup_env();
    e.cost_estimate().budget().reset_unlimited();
    let (contract, _token_client) = setup_contract(&e);

    jump(&e, 1000 + 3601);

    measure(&e, "APPLY FUNDING (1 market)", || {
        e.as_contract(&contract, || {
            trading::trading::execute_apply_funding(&e);
        });
    });
}

#[test]
fn profile_modify_collateral() {
    let e = setup_env();
    e.cost_estimate().budget().reset_unlimited();
    let (contract, token_client) = setup_contract(&e);
    let user = Address::generate(&e);
    token_client.mint(&user, &(10_000_000 * SCALAR_7));

    let id = e.as_contract(&contract, || {
        trading::trading::execute_create_market(
            &e, &user, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, 0, 0, &price_data(&e),
        )
    });

    measure(&e, "MODIFY COLLATERAL (add)", || {
        e.as_contract(&contract, || {
            trading::trading::execute_modify_collateral(&e, id, 1_500 * SCALAR_7, &price_data(&e));
        });
    });
}

#[test]
fn profile_liquidation() {
    let e = setup_env();
    e.cost_estimate().budget().reset_unlimited();
    let (contract, token_client) = setup_contract(&e);
    let user = Address::generate(&e);
    let caller = Address::generate(&e);
    token_client.mint(&user, &(10_000_000 * SCALAR_7));

    let id = e.as_contract(&contract, || {
        trading::trading::execute_create_market(
            &e, &user, 1_100 * SCALAR_7, 100_000 * SCALAR_7, true, 0, 0, &price_data(&e),
        )
    });

    let crash_pd = price_data_at(&e, 9_900_000_000_000_i128);
    measure(&e, "LIQUIDATION", || {
        e.as_contract(&contract, || {
            let requests = vec![&e, ExecuteRequest {
                request_type: ExecuteRequestType::Liquidate as u32,
                position_id: id,
            }];
            trading::trading::execute_trigger(&e, &caller, requests, &crash_pd);
        });
    });
}

#[test]
fn profile_stop_loss() {
    let e = setup_env();
    e.cost_estimate().budget().reset_unlimited();
    let (contract, token_client) = setup_contract(&e);
    let user = Address::generate(&e);
    let caller = Address::generate(&e);
    token_client.mint(&user, &(10_000_000 * SCALAR_7));

    let id = e.as_contract(&contract, || {
        trading::trading::execute_create_limit(
            &e, &user, BTC_FEED_ID, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, BTC_PRICE, 0, 95_000 * PRICE_SCALAR,
        )
    });
    e.as_contract(&contract, || {
        let requests = vec![&e, ExecuteRequest {
            request_type: ExecuteRequestType::Fill as u32,
            position_id: id,
        }];
        trading::trading::execute_trigger(&e, &caller, requests, &price_data(&e));
    });

    jump(&e, 1000 + 31);

    let sl_pd = price_data_at(&e, 9_400_000_000_000_i128);
    measure(&e, "STOP LOSS", || {
        e.as_contract(&contract, || {
            let requests = vec![&e, ExecuteRequest {
                request_type: ExecuteRequestType::StopLoss as u32,
                position_id: id,
            }];
            trading::trading::execute_trigger(&e, &caller, requests, &sl_pd);
        });
    });
}

#[test]
fn profile_take_profit() {
    let e = setup_env();
    e.cost_estimate().budget().reset_unlimited();
    let (contract, token_client) = setup_contract(&e);
    let user = Address::generate(&e);
    let caller = Address::generate(&e);
    token_client.mint(&user, &(10_000_000 * SCALAR_7));

    let id = e.as_contract(&contract, || {
        trading::trading::execute_create_limit(
            &e, &user, BTC_FEED_ID, 1_000 * SCALAR_7, 10_000 * SCALAR_7, true, BTC_PRICE, 110_000 * PRICE_SCALAR, 0,
        )
    });
    e.as_contract(&contract, || {
        let requests = vec![&e, ExecuteRequest {
            request_type: ExecuteRequestType::Fill as u32,
            position_id: id,
        }];
        trading::trading::execute_trigger(&e, &caller, requests, &price_data(&e));
    });

    jump(&e, 1000 + 31);

    let tp_pd = price_data_at(&e, 11_500_000_000_000_i128);
    measure(&e, "TAKE PROFIT", || {
        e.as_contract(&contract, || {
            let requests = vec![&e, ExecuteRequest {
                request_type: ExecuteRequestType::TakeProfit as u32,
                position_id: id,
            }];
            trading::trading::execute_trigger(&e, &caller, requests, &tp_pd);
        });
    });
}
