# Architecture: Threat Modeling for Zenex Perpetual Futures Protocol

**Domain:** DeFi perpetual futures protocol security audit preparation
**Researched:** 2026-03-24
**Confidence:** HIGH (Stellar framework documented, codebase analyzed directly, DeFi threat landscape well-established)

## Recommended Architecture

### Threat Model Document Structure

Follow Stellar's four-question framework (from the [Threat Modeling How-To Guide](https://developers.stellar.org/docs/build/security-docs/threat-modeling/threat-modeling-how-to)) adapted for a multi-contract DeFi protocol. The threat model should be a single living document organized as follows:

```
docs/threat-model/
  THREAT-MODEL.md          # Master document (four Stellar questions)
  data-flow-diagrams/
    dfd-overview.md         # System-level DFD with trust boundaries
    dfd-position-lifecycle.md  # Open -> fill -> close/liquidate
    dfd-funding-borrowing.md   # Rate accrual and settlement
    dfd-adl.md              # Auto-deleveraging flow
    dfd-governance.md       # Timelock config update flow
  stride/
    STRIDE-ANALYSIS.md      # All STRIDE threats with IDs (Spoof.1, Tamp.1, etc.)
    STRIDE-MITIGATIONS.md   # Treatment for each threat
  test-mapping/
    THREAT-TEST-MAP.md      # Threat ID -> test case mapping
```

### Section 1: "What Are We Working On?" (Data Flow Diagrams)

**Required diagrams for Zenex:**

**DFD 1 -- System Overview:**

```
+------------------+     +------------------+     +------------------+
|  External Actors |     |   Trust Boundary |     |  Contract System |
+------------------+     +------------------+     +------------------+
                         |                  |
  [User/Trader] -------->| require_auth()   |-----> [Trading Contract]
                         |                  |         |    |    |
  [Keeper/Bot] --------->| caller.require   |----+--->+    |    |
                         |  _auth()         |    |        |    |
  [Relayer] ------------>| tx submission    |----+        |    |
                         |  (no auth check) |             |    |
  [Admin/Owner] -------->| #[only_owner]    |--->+        |    |
                         |                  |    |        |    |
  [Governance] --------->| timelock delay   |--->+--->set_config/
                         |                  |         set_market
                         +------------------+
                                                      |    |
                                                      v    v
                         +------------------+    +----------+--------+
                         | Trust Boundary   |    | Cross-Contract    |
                         +------------------+    +-------------------+
                         |                  |    |                   |
  [Pyth Lazer Signer] ->| ed25519_verify() |--->| PriceVerifier     |
                         | staleness check  |    | .verify_prices()  |
                         | confidence check |    +-------------------+
                         +------------------+           |
                                                        v
                         +------------------+    +-------------------+
                         | Trust Boundary   |    |                   |
                         +------------------+    | Strategy Vault    |
                         |                  |    | .total_assets()   |
  [LP Depositors] ----->| lock_time gate   |--->| .strategy_withdraw|
                         | require_auth()   |    +-------------------+
                         +------------------+           |
                                                        v
                                                 +-------------------+
                                                 | Treasury          |
                                                 | .get_fee()        |
                                                 | .get_rate()       |
                                                 +-------------------+
```

**DFD 2 -- Position Lifecycle (most attack-prone flow):**

```
User submits open_market(user, feed_id, collateral, notional, is_long, tp, sl, price_bytes)
  |
  v
[require_active] -- status must be Active (not OnIce/Frozen)
  |
  v
[user.require_auth()] -- Stellar signature verification
  |
  v
[PriceVerifier.verify_prices(price_bytes)] -- TRUST BOUNDARY: Pyth data
  |-- ed25519_verify(pubkey, payload, sig) against trusted_signer
  |-- staleness check: |now - publish_time| <= max_staleness
  |-- confidence check: confidence * 10000 <= price * max_confidence_bps
  |-- feed_id match check
  |
  v
[Market::load] -- Fetches config, data, vault_balance, accrues indices
  |-- VaultClient.total_assets() -- TRUST BOUNDARY: cross-contract
  |-- MarketData.accrue() -- time-weighted borrowing + funding
  |
  v
[Position::create + validate] -- bounds checks
  |-- notional in [min_notional, max_notional]
  |-- leverage <= 1/margin
  |-- collateral > 0
  |-- market enabled
  |
  v
[Market::open] -- fee computation + fill
  |-- base_fee = notional * fee_rate (dominant vs non-dominant)
  |-- impact_fee = notional / impact_scalar
  |-- position.col -= (base_fee + impact_fee)
  |-- re-validate after fee deduction
  |-- position.fill() -- snapshot indices
  |-- update_stats -- update notionals, entry weights
  |-- require_within_util -- global + per-market cap check
  |
  v
[Token transfers]
  |-- user -> contract: collateral
  |-- contract -> vault: vault_fee
  |-- contract -> treasury: treasury_fee
  |
  v
[Event emission + storage persistence]
```

**DFD 3 -- Keeper Execution (Fill / SL / TP / Liquidate):**

```
Keeper submits execute(caller, requests: Vec<ExecuteRequest>, price_bytes)
  |
  v
[require_can_manage] -- Active, OnIce, or AdminOnIce (NOT Frozen)
  |
  v
[caller.require_auth()] -- keeper identity (for fee payout)
  |-- NOTE: Any address can be caller. No allowlist.
  |-- Keeper receives caller_rate share of trading fees.
  |
  v
[verify_price] -- single price feed for the batch
  |-- ALL requests in batch must match this feed_id
  |
  v
[For each request in batch]:
  |
  +-- Fill: position.filled == false, price meets limit condition
  |     long: market.price <= position.entry_price
  |     short: market.price >= position.entry_price
  |
  +-- StopLoss: position.check_stop_loss(price) == true
  |     long: price <= sl; short: price >= sl
  |
  +-- TakeProfit: position.check_take_profit(price) == true
  |     long: price >= tp; short: price <= tp
  |
  +-- Liquidate: equity < notional * liq_fee
  |     position gets nothing; vault absorbs residual
  |
  v
[Transfers aggregated via Transfers map]:
  vault_transfer < 0 -> vault pays contract (strategy_withdraw)
  user_payout > 0 -> contract pays user
  treasury_fee > 0 -> contract pays treasury
  caller_fee > 0 -> contract pays keeper
```

### Section 2: "What Can Go Wrong?" (STRIDE Analysis)

Each threat must be uniquely identified per the [STRIDE Template](https://developers.stellar.org/docs/build/security-docs/threat-modeling/STRIDE-template). Below is the complete STRIDE analysis for Zenex.

---

#### S -- Spoofing (Authentication Bypass)

| ID | Threat | Component | Severity |
|----|--------|-----------|----------|
| Spoof.1 | Keeper submits execute() with crafted price data that passes verification but benefits specific positions | Trading + PriceVerifier | CRITICAL |
| Spoof.2 | Attacker replays old Pyth price update that is still within max_staleness window but significantly different from current market price | PriceVerifier | HIGH |
| Spoof.3 | Compromised Pyth Lazer trusted_signer key signs fabricated prices | PriceVerifier | CRITICAL |
| Spoof.4 | Attacker calls admin functions on trading contract directly (bypassing governance timelock) if trading contract owner is not set to governance | Trading/Governance | HIGH |
| Spoof.5 | Malicious contract impersonates vault or price verifier (addresses stored immutably at construction but no runtime verification of contract code) | Trading | MEDIUM |
| Spoof.6 | User's position acted upon by keeper without the user's consent (by design for liquidation/SL/TP -- this is expected behavior, not a vulnerability, but must be documented) | Trading | LOW (by design) |

---

#### T -- Tampering (Data Integrity)

| ID | Threat | Component | Severity |
|----|--------|-----------|----------|
| Tamp.1 | Fixed-point arithmetic rounding manipulation: attacker opens/closes positions at specific sizes to exploit floor/ceil rounding in fee calculations, accumulating dust over many transactions | Trading (position.rs, market.rs) | HIGH |
| Tamp.2 | Funding index manipulation: attacker opens large one-sided position to skew funding rate, then opens opposing position on second account to collect funding payments | Trading (rates.rs, market.rs) | HIGH |
| Tamp.3 | ADL index manipulation: attacker triggers ADL at a favorable moment to reduce their winning position's notional while retaining profit from the index ratio calculation | Trading (adl.rs) | HIGH |
| Tamp.4 | Entry weight desynchronization: rapid open/close cycles cause l_entry_wt or s_entry_wt to drift from actual aggregate due to rounding in fixed_div_floor | Trading (market.rs) | MEDIUM |
| Tamp.5 | Utilization gaming: attacker opens positions up to max_util then manipulates vault_balance via vault deposits/withdrawals to temporarily exceed intended utilization limits | Trading + Vault | MEDIUM |
| Tamp.6 | Governance queued update frontrunning: attacker sees pending config update and positions before the change takes effect (e.g., fee reduction queued, open positions before it applies) | Governance | LOW |
| Tamp.7 | Position collateral modification race: user calls modify_collateral to reduce collateral while simultaneously being liquidated by a keeper in the same ledger | Trading (actions.rs) | MEDIUM |
| Tamp.8 | Borrowing rate spike: when vault_balance drops (LP withdrawals), utilization spikes, causing retroactive borrowing rate increase for existing positions that accrues on next Market::load | Trading (market.rs) | MEDIUM |

---

#### R -- Repudiation (Non-Denial)

| ID | Threat | Component | Severity |
|----|--------|-----------|----------|
| Rep.1 | Events are the only audit trail; if event emission is skipped (bug or gas limit), position actions become non-provable | Trading (events.rs) | MEDIUM |
| Rep.2 | ADL reduces position notionals without individual position consent; affected users may dispute the reduction. ADLTriggered event only logs aggregate reduction_pct, not per-position breakdown | Trading (adl.rs) | MEDIUM |
| Rep.3 | Funding rate changes occur globally via apply_funding() without per-position event trail; users cannot independently verify their specific funding charge from events alone | Trading (actions.rs) | LOW |
| Rep.4 | Governance timelock uses temporary storage which has TTL expiration. A queued config update could expire before execution, with no event logged for the expiration itself | Governance | LOW |

---

#### I -- Information Disclosure

| ID | Threat | Component | Severity |
|----|--------|-----------|----------|
| Info.1 | All position data (user address, collateral, notional, entry price, SL/TP) is readable on-chain via getter functions. Competitors/liquidators can front-run SL/TP triggers | Trading | MEDIUM |
| Info.2 | Vault balance (total_assets) is publicly readable. Combined with position data, any observer can compute exact liquidation prices for every position | Trading + Vault | MEDIUM |
| Info.3 | Queued governance updates are readable via get_queued_config and get_queued_market, allowing adversaries to pre-position for parameter changes | Governance | LOW |
| Info.4 | Event data includes full settlement breakdown (pnl, fees, funding, borrowing) for every position close/liquidation, revealing trading strategy details | Trading (events.rs) | LOW |

---

#### D -- Denial of Service

| ID | Threat | Component | Severity |
|----|--------|-----------|----------|
| DoS.1 | Keeper liveness failure: if no keeper calls execute() or apply_funding(), positions cannot be liquidated, SL/TP cannot trigger, and funding/borrowing indices become stale | Trading | CRITICAL |
| DoS.2 | Oracle liveness failure: if Pyth Lazer stops producing signed prices, no market orders can open, no positions can close, update_status cannot fire | PriceVerifier | CRITICAL |
| DoS.3 | Vault griefing: attacker deposits minimal amount to vault to prevent total_assets from being zero (avoiding division-by-zero), but keeps balance so low that all utilization checks fail for meaningful positions | Vault | MEDIUM |
| DoS.4 | Admin freezes contract (set_status to Frozen) maliciously or under duress; no positions can be managed | Trading | HIGH |
| DoS.5 | MAX_ENTRIES (50) user position cap: attacker opens 50 minimum-size positions per user address, blocking further position creation for that user (but new addresses are free on Stellar) | Trading (storage) | LOW |
| DoS.6 | Storage TTL expiration: if trading contract instance or position persistent storage entries are not bumped within their TTL windows, data becomes inaccessible | Trading (storage) | LOW |
| DoS.7 | Batch execute with large Vec<ExecuteRequest> could exceed Soroban CPU/memory budget, causing transaction failure | Trading (execute.rs) | MEDIUM |
| DoS.8 | apply_funding iterates over ALL markets in a single transaction; if market count grows large, the transaction may exceed resource limits | Trading (actions.rs) | MEDIUM |

---

#### E -- Elevation of Privilege

| ID | Threat | Component | Severity |
|----|--------|-----------|----------|
| EoP.1 | Owner key compromise: owner can set_config, set_market, del_market, set_status without timelock when governance is not the trading contract owner | Trading | CRITICAL |
| EoP.2 | Governance bypass: if trading contract owner is an EOA (not governance contract), timelock can be circumvented entirely | Trading + Governance | CRITICAL |
| EoP.3 | Strategy-only vault withdrawal: if trading contract address is compromised or upgraded maliciously, attacker can drain vault via strategy_withdraw | Vault | CRITICAL |
| EoP.4 | Upgradeable contracts: both TradingContract and GovernanceContract derive Upgradeable. A malicious upgrade can replace all contract logic | Trading + Governance | CRITICAL |
| EoP.5 | Governance set_status is immediate (no timelock). Admin can freeze trading instantly, trapping user funds in positions that cannot be managed | Governance | HIGH |
| EoP.6 | Price verifier owner can change trusted_signer, max_staleness, and max_confidence_bps. A compromised verifier owner could relax staleness to accept arbitrarily old prices | PriceVerifier | HIGH |
| EoP.7 | Factory deploy has no access control beyond constructor admin parameter. Anyone who can call deploy gets a new trading+vault pair | Factory | LOW |
| EoP.8 | Treasury owner can set_rate to 100% (SCALAR_7), diverting all protocol fees to treasury, though this does not affect user PnL | Treasury | MEDIUM |
| EoP.9 | Keeper caller_rate up to 50% (MAX_CALLER_RATE = 5_000_000). Combined with a colluding admin setting this to max, keeper extracts half of all trading fees | Trading | MEDIUM |

---

### Section 3: "What Are We Going To Do About It?" (Mitigations)

For each threat, map to existing mitigations (already in code) and required mitigations (need tests or documentation).

| Threat ID | Existing Mitigation | Required Mitigation / Test |
|-----------|--------------------|-----------------------------|
| Spoof.1 | ed25519_verify + trusted_signer check in pyth.rs | Test: submit price_data with wrong signer, stale price, wrong feed_id |
| Spoof.2 | max_staleness config in price verifier | Test: edge case at exactly max_staleness boundary; document recommended max_staleness value |
| Spoof.3 | None (external trust assumption) | Document: trusted_signer rotation procedure, monitoring for Pyth key compromise |
| Spoof.4 | #[only_owner] on admin functions | Test: verify governance is set as trading owner in deployment; test direct admin call rejection |
| Spoof.5 | Addresses set at construction, immutable | Document: deployment verification checklist (verify contract WASM hashes at deploy time) |
| Tamp.1 | ceil rounding on fees (attacker pays more, not less) | Test: dust accumulation over 1000+ open/close cycles; verify no value leakage |
| Tamp.2 | Funding rate bounded by r_funding base rate | Test: large one-sided position funding rate calculation; verify P2P conservation |
| Tamp.3 | ADL factor capped at SCALAR_18 (100% reduction max) | Test: ADL with multiple markets, verify per-position notional reduction accuracy |
| Tamp.4 | None explicit | Test: entry_wt consistency after rapid open/close sequences; document acceptable drift |
| Tamp.5 | require_within_util checked at position open | Test: deposit to vault, open position at util limit, withdraw from vault, verify no new opens possible |
| Tamp.6 | Timelock delay | Document: recommended monitoring for pending governance updates |
| Tamp.7 | Soroban's single-threaded execution (no concurrent tx on same account) | Document: Soroban execution model prevents true race conditions within a ledger |
| Tamp.8 | accrue() uses time-weighted rates | Test: borrowing rate increase after vault withdrawal; verify no retroactive penalty beyond elapsed time |
| DoS.1 | apply_funding is permissionless (anyone can call) | Test: verify no auth required on apply_funding; document keeper failover strategy |
| DoS.2 | None (external dependency) | Document: fallback procedures if oracle is down; admin can freeze to protect vault |
| DoS.3 | require_within_util checks vault_balance > 0 | Test: zero-balance vault rejection; tiny-balance vault behavior |
| DoS.4 | Governance timelock on config changes (but set_status is immediate) | Document: emergency procedure documentation; consider requiring multi-sig for Frozen |
| DoS.7 | None explicit | Test: determine maximum safe batch size within Soroban resource limits |
| DoS.8 | MAX_ENTRIES = 50 markets | Test: apply_funding with 50 markets; verify within resource budget |
| EoP.1 | #[only_owner] macro | Test: non-owner cannot call admin functions; verify owner transfer procedure |
| EoP.2 | Governance contract exists with timelock | Document: deployment must set governance as trading owner |
| EoP.3 | strategy.require_auth() in vault | Test: non-strategy address cannot call strategy_withdraw |
| EoP.4 | UpgradeableInternal requires owner auth | Document: upgrade procedure, WASM hash verification, multi-sig requirement |
| EoP.5 | set_status has no timelock by design (emergency use) | Document: justification for immediate status changes; monitoring requirements |
| EoP.6 | #[only_owner] on verifier config updates | Test: non-owner cannot update signer/staleness; document verifier owner setup |
| EoP.8 | Treasury rate has no explicit cap beyond SCALAR_7 | Test: verify treasury rate cannot exceed 100% (SCALAR_7) |
| EoP.9 | MAX_CALLER_RATE = 50% cap | Test: verify caller_rate validation in require_valid_config |

### Section 4: "Did We Do A Good Job?" (Retrospective Checklist)

Apply after completing the threat model document:

- [ ] DFD diagrams have been referenced by team members beyond the author
- [ ] At least one threat identified in each STRIDE category (verified above: S=6, T=8, R=4, I=4, D=8, E=9)
- [ ] Every CRITICAL threat has a mitigation with corresponding test case
- [ ] Previously unconsidered design concerns were surfaced (entry_wt drift, governance TTL expiry, batch resource limits)
- [ ] Threat model reviewed after any contract modification (currently code-frozen, but applies post-audit)

---

## Trust Boundaries

Trust boundaries are the architectural seams where one system trusts another. Each boundary requires explicit documentation of what is trusted and verification mechanisms.

### Boundary 1: User -> Trading Contract

| Aspect | Detail |
|--------|--------|
| **Verification** | `user.require_auth()` via Stellar's Ed25519/WebAuthn signature scheme |
| **What's trusted** | User identity via cryptographic signature |
| **What's NOT trusted** | All user inputs (collateral, notional, prices) validated by contract |
| **Attack surface** | User can submit any parameters; contract must enforce all bounds |
| **Key invariant** | Only position owner can close/modify/cancel their position |

### Boundary 2: Keeper -> Trading Contract

| Aspect | Detail |
|--------|--------|
| **Verification** | `caller.require_auth()` for keeper fee payout; no allowlist |
| **What's trusted** | Keeper identity (for fee payment) |
| **What's NOT trusted** | Keeper's choice of which positions to execute, timing, price data |
| **Attack surface** | Any address can be keeper. Keeper selects which positions to liquidate/fill/trigger. Keeper submits signed price data. |
| **Key invariants** | Liquidation requires position to be underwater; SL/TP require trigger conditions; fill requires limit conditions; all require valid signed price |
| **Critical concern** | Keeper can selectively liquidate, potentially griefing specific users by choosing unfavorable timing |

### Boundary 3: Trading Contract -> Price Verifier

| Aspect | Detail |
|--------|--------|
| **Verification** | `ed25519_verify(pubkey, payload, sig)` on Pyth Lazer data |
| **What's trusted** | Pyth Network's signing infrastructure (trusted_signer public key) |
| **What's NOT trusted** | The raw bytes submitted (fully verified before use) |
| **Attack surface** | Stale prices within max_staleness window; confidence interval edge cases; Pyth key compromise |
| **Key invariants** | Price must be from trusted_signer, within staleness, within confidence bounds, correct feed_id |

### Boundary 4: Trading Contract -> Strategy Vault

| Aspect | Detail |
|--------|--------|
| **Verification** | `strategy.require_auth()` in vault's strategy_withdraw |
| **What's trusted** | Trading contract address (set at vault construction as "strategy") |
| **What's NOT trusted** | Withdrawal amounts (but no explicit cap in vault -- trusts strategy to withdraw correct amounts) |
| **Attack surface** | Compromised/upgraded trading contract can drain vault |
| **Key invariants** | Only the designated strategy (trading contract) can call strategy_withdraw; vault balance must remain >= 0 |
| **Critical concern** | No withdrawal cap in vault -- trading contract could theoretically withdraw more than a position's entitlement if settlement math has a bug |

### Boundary 5: Trading Contract -> Treasury

| Aspect | Detail |
|--------|--------|
| **Verification** | Treasury.get_fee() is a pure calculation; no auth required to read |
| **What's trusted** | Treasury rate is correctly configured |
| **What's NOT trusted** | Treasury rate value (admin-controlled) |
| **Attack surface** | Malicious rate could divert all protocol fees |
| **Key invariant** | Treasury fees come from protocol fees only, never from user collateral or PnL directly |

### Boundary 6: Governance -> Trading Contract

| Aspect | Detail |
|--------|--------|
| **Verification** | Timelock delay (queue -> wait -> execute); owner auth on queue/cancel |
| **What's trusted** | Governance owner's intent (after delay period) |
| **What's NOT trusted** | Config values (validated by trading contract's require_valid_config) |
| **Attack surface** | set_status is immediate (no timelock) -- emergency power; queued updates use temporary storage that can expire |
| **Key invariants** | Config changes cannot bypass timelock; status changes are immediate by design; expired queued updates are harmlessly lost |

### Boundary 7: LP Depositors -> Vault

| Aspect | Detail |
|--------|--------|
| **Verification** | `require_auth()` for deposits; lock_time gate for withdrawals |
| **What's trusted** | Depositor identity, deposit amounts |
| **What's NOT trusted** | Withdrawal timing (locked for lock_time seconds) |
| **Attack surface** | Vault inflation attack (mitigated by decimals_offset); share price manipulation via flash deposits |
| **Key invariant** | Lock time prevents flash deposit/withdraw attacks; decimals_offset prevents inflation attack |

---

## Patterns to Follow

### Pattern 1: Threat-to-Test Mapping Matrix

Every identified threat must have at least one corresponding test case. Use a structured mapping format.

**Format:**
```markdown
| Threat ID | Test File | Test Function | Verification |
|-----------|-----------|---------------|--------------|
| Spoof.1 | test-suites/src/price_verifier.rs | test_reject_wrong_signer | Price with invalid signer is rejected |
| Spoof.1 | test-suites/src/price_verifier.rs | test_reject_stale_price | Price beyond max_staleness is rejected |
| Tamp.1 | test-suites/src/fee_arithmetic.rs | test_dust_accumulation_1000_cycles | Sum of fees after 1000 trades matches expected within 1 unit |
| EoP.3 | test-suites/src/vault_security.rs | test_unauthorized_strategy_withdraw | Non-strategy caller is rejected |
```

**Test categories derived from STRIDE:**
- **Spoofing tests**: Verify auth requirements, signature validation, identity checks
- **Tampering tests**: Verify arithmetic invariants, index consistency, conservation of value
- **Repudiation tests**: Verify event emission on every state change
- **Information disclosure tests**: (Limited scope on-chain -- focus on documenting what's public)
- **DoS tests**: Verify resource limits, permissionless function availability, edge cases
- **EoP tests**: Verify access control on every admin/privileged function

### Pattern 2: Invariant-Based Testing

For each contract, define mathematical invariants that must hold across all state transitions:

**Trading Contract Invariants:**
```
INV-1: total_notional == sum(market.l_notional + market.s_notional) for all markets
INV-2: For any position close: user_payout + vault_transfer + treasury_fee + caller_fee == position.col
INV-3: Funding is zero-sum P2P: sum(funding_paid_by_longs) == sum(funding_received_by_shorts) (and vice versa)
INV-4: After ADL: reduced_notional = original_notional * factor / SCALAR_18 where factor <= SCALAR_18
INV-5: vault_balance >= sum(user_payouts) for all profitable positions (enforced by ADL circuit breaker)
INV-6: position.col > notional * margin after fees (enforced at open, may degrade during position lifetime)
INV-7: No position can be liquidated within MIN_OPEN_TIME (30s) of creation
```

**Vault Invariants:**
```
INV-V1: total_assets() >= sum of all token balances held by vault contract
INV-V2: Only strategy address can call strategy_withdraw
INV-V3: Depositor cannot withdraw before lock_time expires
INV-V4: Share price monotonically increases (absent strategy withdrawals)
```

### Pattern 3: Conservation of Value Testing

For every token transfer path, verify that tokens are neither created nor destroyed:

```
Test template:
  1. Record all token balances (user, contract, vault, treasury)
  2. Execute action (open, close, liquidate, etc.)
  3. Record all token balances again
  4. Assert: sum(before) == sum(after)
```

This catches bugs where rounding, incorrect fee calculations, or missing transfers create or destroy value.

### Pattern 4: Edge Case Boundary Testing

For every numeric boundary in the protocol, test at, above, and below:

| Boundary | Value | Test At | Test Above | Test Below |
|----------|-------|---------|------------|------------|
| max_staleness | configurable | Accept | Reject | Accept |
| max_util (global) | configurable | Accept | Reject | Accept |
| max_util (market) | configurable | Accept | Reject | Accept |
| margin requirement | configurable | Accept (exactly) | Accept (over-collateralized) | Reject (under) |
| liq_fee threshold | configurable | Not liquidatable | N/A | Liquidatable |
| MIN_OPEN_TIME | 30s | Closable | Closable | Not closable |
| ONE_HOUR_SECONDS | 3600 | Can fund | Can fund | Cannot fund |
| UTIL_ONICE | 95% | Triggers ADL | Triggers ADL | Threshold not met |
| UTIL_ACTIVE | 90% | Remains OnIce | Remains OnIce | Restores Active |

---

## Anti-Patterns to Avoid

### Anti-Pattern 1: Testing Contracts in Isolation

**What:** Testing each contract's functions individually without cross-contract interaction.
**Why bad:** Most Zenex vulnerabilities exist at trust boundaries between contracts. A settlement calculation that is correct in isolation may drain the vault when combined with a specific market state.
**Instead:** Integration tests in test-suites must deploy the full contract stack (factory -> vault + trading + price verifier + treasury) and test end-to-end flows.

### Anti-Pattern 2: Happy-Path-Only Testing

**What:** Only testing that correct inputs produce correct outputs.
**Why bad:** Attackers never use correct inputs. The threat model's value comes from identifying what happens with adversarial inputs.
**Instead:** For every test case, write a corresponding adversarial test. If `test_open_market_long` tests normal opening, write `test_open_market_wrong_feed_id`, `test_open_market_stale_price`, `test_open_market_max_leverage`, `test_open_market_zero_collateral`.

### Anti-Pattern 3: Assuming Soroban Prevents All Concurrency Issues

**What:** Believing that Soroban's single-threaded execution eliminates all race conditions.
**Why bad:** While true within a single ledger close, cross-ledger ordering is still adversarial. A keeper can see a user's pending close transaction and front-run with a liquidation in the same ledger.
**Instead:** Document the expected behavior when conflicting actions target the same position in the same ledger. Test that the first-processed action succeeds and the second correctly fails.

### Anti-Pattern 4: Trusting External Dependencies Without Documenting Trust Assumptions

**What:** Not documenting what happens if Pyth, the relayer, or Stellar itself has an outage or provides incorrect data.
**Why bad:** Auditors will ask. Undocumented assumptions become critical findings.
**Instead:** For each external dependency, document: what's trusted, what happens if it fails, what the fallback is, and what monitoring detects the failure.

---

## Scalability Considerations

| Concern | Current (Testnet) | At Production Launch | Long-Term |
|---------|-------------------|---------------------|-----------|
| Market count | ~5 markets | Up to MAX_ENTRIES (50) | apply_funding resource limits may need batching |
| Positions per market | Low hundreds | Thousands | Position storage TTL management becomes critical |
| Keeper liveness | Single keeper | Need 2+ redundant keepers | Consider on-chain incentives for keeper diversity |
| Oracle freshness | max_staleness = generous | Tighten to 30-60s | Multiple oracle sources (Pyth + backup) |
| Governance delay | Short for testing | Minimum 48h recommended | Consider time-weighted voting for parameter changes |
| Vault size | Small (testnet tokens) | $1M-$10M TVL | ADL thresholds may need adjustment; consider partial ADL |

---

## Audit Documentation Organization

Recommended structure for the complete audit package:

```
docs/
  threat-model/           # This research -> formalized
    THREAT-MODEL.md       # Master Stellar 4-question document
    STRIDE-ANALYSIS.md    # Complete STRIDE table with IDs
    MITIGATIONS.md        # Treatment per threat
    THREAT-TEST-MAP.md    # Threat -> test case mapping
  protocol/
    SPEC.md               # Protocol specification (math, flows, invariants)
    ARCHITECTURE.md       # System architecture for auditors
    API-REFERENCE.md      # Every entry point documented
  deployment/
    DEPLOYMENT-GUIDE.md   # How to deploy safely
    VERIFICATION.md       # Post-deployment verification steps
```

**Auditor-facing priority:**
1. Threat model (tells auditors what to focus on)
2. Protocol spec (tells auditors what correct behavior is)
3. Test suite (tells auditors what's already verified)
4. Architecture docs (tells auditors how pieces fit together)

---

## Sources

### Stellar/Soroban Security Framework
- [Stellar Threat Modeling Overview](https://developers.stellar.org/docs/build/security-docs/threat-modeling) -- STRIDE framework reference (HIGH confidence)
- [Threat Modeling How-To Guide](https://developers.stellar.org/docs/build/security-docs/threat-modeling/threat-modeling-how-to) -- Four-question framework (HIGH confidence)
- [STRIDE Template](https://developers.stellar.org/docs/build/security-docs/threat-modeling/STRIDE-template) -- Template structure (HIGH confidence)
- [Soroban Security Audit Bank](https://stellar.org/grants-and-funding/soroban-audit-bank) -- SDF audit program (HIGH confidence)
- [Veridise Soroban Security Checklist](https://veridise.com/blog/audit-insights/building-on-stellar-soroban-grab-this-security-checklist-to-avoid-vulnerabilities/) -- Soroban-specific vulnerabilities (HIGH confidence)

### Perpetual DEX Security
- [Hacken: Perpetual DEX Security Evolution](https://hacken.io/discover/perpetual-dex-security-evolution/) -- Historical attack vectors, oracle manipulation, economic attacks (HIGH confidence)
- [Stellar Audited Projects](https://stellar.org/audit-bank/projects) -- Reference for audit scope (MEDIUM confidence)

### Codebase Analysis
- Direct code review of trading, vault, price-verifier, governance, factory, treasury contracts (HIGH confidence)
- All STRIDE threats derived from actual code paths, not theoretical abstractions

---

*Architecture research: 2026-03-24*
