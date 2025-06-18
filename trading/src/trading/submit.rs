use soroban_sdk::{vec, Address, Env, IntoVal, Symbol, Val, Vec};
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::token::TokenClient;
use crate::dependencies::VaultClient;
use crate::storage;
use crate::trading::{Request};
use crate::trading::actions::build_actions_from_request;
use crate::trading::trading::Trading;

pub fn execute_submit(
    e: &Env,
    caller: &Address,
    requests: Vec<Request>,
) -> i128 {
    let mut trading = Trading::load(e);
    let mut actions = build_actions_from_request(&e, &mut trading, requests, caller.clone());
    let token_client = TokenClient::new(&e, &storage::get_token(e));
    let vault_client = VaultClient::new(&e, &storage::get_vault(e));
    
    //First, make sure the vault has enough funds to payoff
    if actions.vault_transfer > 0 {
        vault_client.transfer_to(&e.current_contract_address(), &actions.vault_transfer);
    } else if actions.vault_transfer < 0 {
        // Craft auth
        let args: Vec<Val> = vec![
            e,
            e.current_contract_address().into_val(e),
            vault_client.address.into_val(e),
            (-actions.vault_transfer).into_val(e),
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
        vault_client.transfer_from(&e.current_contract_address(), &-actions.vault_transfer);
    }
    for (owner, amount) in actions.owner_transfers.iter() {
        if owner == *caller {
            actions.add_for_spender_transfer(amount);
        } else {
            token_client.transfer(&e.current_contract_address(), &owner, &amount);
        }
    }
    
    token_client.transfer(&e.current_contract_address(), &caller, &actions.spender_transfer);
    actions.spender_transfer
}