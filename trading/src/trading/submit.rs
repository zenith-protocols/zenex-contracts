use soroban_sdk::{vec, Address, Env, IntoVal, Symbol, Val, Vec};
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::token::TokenClient;
use crate::dependencies::VaultClient;
use crate::storage;
use crate::trading::Request;
use crate::trading::actions::{process_requests, SubmitResult};
use crate::trading::core::Trading;

pub fn execute_submit(
    e: &Env,
    caller: &Address,
    requests: Vec<Request>,
) -> SubmitResult {
    let mut trading = Trading::load(e, caller.clone());
    let result = process_requests(e, &mut trading, requests);

    let token_client = TokenClient::new(e, &storage::get_token(e));
    let vault_client = VaultClient::new(e, &storage::get_vault(e));

    // STEP 1: Vault pays to contract (if needed)
    // This is done first to ensure the contract has enough balance to handle transfers
    let vault_transfer = result.transfers.get(trading.vault.clone()).unwrap_or(0);
    if vault_transfer < 0 {
        vault_client.transfer_to(&e.current_contract_address(), &vault_transfer.abs());
    }

    // STEP 2: Handle all other transfers (users)
    for (address, amount) in result.transfers.iter() {
        if address != trading.vault {
            if amount > 0 {
                // Contract pays to user
                token_client.transfer(&e.current_contract_address(), &address, &amount);
            } else if amount < 0 {
                // User pays to contract
                token_client.transfer(&address, &e.current_contract_address(), &(-amount));
            }
        }
    }

    // STEP 3: Contract pays to vault if needed
    // This is done last to ensure the contract has enough balance
    if vault_transfer > 0 {
        let args: Vec<Val> = vec![
            e,
            e.current_contract_address().into_val(e),
            vault_client.address.into_val(e),
            vault_transfer.into_val(e),
        ];
        e.authorize_as_current_contract(vec![
            e,
            InvokerContractAuthEntry::Contract(SubContractInvocation {
                context: ContractContext {
                    contract: token_client.address.clone(),
                    fn_name: Symbol::new(e, "transfer"),
                    args: args.clone(),
                },
                sub_invocations: vec![e],
            })
        ]);
        vault_client.transfer_from(&e.current_contract_address(), &vault_transfer);
    }

    result
}