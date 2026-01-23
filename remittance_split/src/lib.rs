#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token::TokenClient, vec, Address, Env, Symbol,
    Vec,
};

#[derive(Clone)]
#[contracttype]
pub struct Allocation {
    pub category: Symbol,
    pub amount: i128,
}

#[contract]
pub struct RemittanceSplit;

#[contractimpl]
impl RemittanceSplit {
    /// Initialize a remittance split configuration
    pub fn initialize_split(
        env: Env,
        spending_percent: u32,
        savings_percent: u32,
        bills_percent: u32,
        insurance_percent: u32,
    ) -> bool {
        let total = spending_percent + savings_percent + bills_percent + insurance_percent;

        if total != 100 {
            return false;
        }

        env.storage().instance().set(
            &symbol_short!("SPLIT"),
            &vec![
                &env,
                spending_percent,
                savings_percent,
                bills_percent,
                insurance_percent,
            ],
        );

        true
    }

    /// Get the current split configuration
    pub fn get_split(env: &Env) -> Vec<u32> {
        env.storage()
            .instance()
            .get(&symbol_short!("SPLIT"))
            .unwrap_or_else(|| vec![env, 50, 30, 15, 5])
    }

    /// Calculate split amounts from a total remittance amount
    pub fn calculate_split(env: Env, total_amount: i128) -> Vec<i128> {
        let split = Self::get_split(&env);

        let spending = (total_amount * split.get(0).unwrap() as i128) / 100;
        let savings = (total_amount * split.get(1).unwrap() as i128) / 100;
        let bills = (total_amount * split.get(2).unwrap() as i128) / 100;
        let insurance = total_amount - spending - savings - bills;

        vec![&env, spending, savings, bills, insurance]
    }

    /// Distribute USDC according to the configured split
    pub fn distribute_usdc(
        env: Env,
        usdc_contract: Address,
        from: Address,
        spending_account: Address,
        savings_account: Address,
        bills_account: Address,
        insurance_account: Address,
        total_amount: i128,
    ) -> bool {
        if total_amount <= 0 {
            return false;
        }

        from.require_auth();

        let amounts = Self::calculate_split(env.clone(), total_amount);
        let recipients = [
            spending_account,
            savings_account,
            bills_account,
            insurance_account,
        ];
        let token = TokenClient::new(&env, &usdc_contract);

        for (amount, recipient) in amounts.into_iter().zip(recipients.iter()) {
            if amount > 0 {
                token.transfer(&from, recipient, &amount);
            }
        }

        true
    }

    /// Query USDC balance for an address
    pub fn get_usdc_balance(env: &Env, usdc_contract: Address, account: Address) -> i128 {
        TokenClient::new(env, &usdc_contract).balance(&account)
    }

    /// Returns a breakdown of the split by category and resulting amount
    pub fn get_split_allocations(env: &Env, total_amount: i128) -> Vec<Allocation> {
        let amounts = Self::calculate_split(env.clone(), total_amount);
        let categories = [
            symbol_short!("SPENDING"),
            symbol_short!("SAVINGS"),
            symbol_short!("BILLS"),
            symbol_short!("INSURANCE"),
        ];

        let mut result = Vec::new(env);
        for (category, amount) in categories.into_iter().zip(amounts.into_iter()) {
            result.push_back(Allocation {
                category,
                amount,
            });
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, token::{StellarAssetClient, TokenClient}, Env};

    #[test]
    fn distribute_usdc_apportions_tokens_to_recipients() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, RemittanceSplit);
        let client = RemittanceSplitClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let token_contract = env.register_stellar_asset_contract_v2(admin.clone());
        let payer = Address::generate(&env);
        let amount = 1_000i128;

        StellarAssetClient::new(&env, &token_contract.address()).mint(&payer, &amount);

        let spending = Address::generate(&env);
        let savings = Address::generate(&env);
        let bills = Address::generate(&env);
        let insurance = Address::generate(&env);

        let distributed = client.distribute_usdc(
            &token_contract.address(),
            &payer,
            &spending,
            &savings,
            &bills,
            &insurance,
            &amount,
        );

        assert!(distributed);

        let token_client = TokenClient::new(&env, &token_contract.address());
        assert_eq!(token_client.balance(&spending), 500);
        assert_eq!(token_client.balance(&savings), 300);
        assert_eq!(token_client.balance(&bills), 150);
        assert_eq!(token_client.balance(&insurance), 50);
        assert_eq!(token_client.balance(&payer), 0);
    }

    #[test]
    fn split_allocations_report_categories_and_amounts() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, RemittanceSplit);
        let client = RemittanceSplitClient::new(&env, &contract_id);

        let total_amount = 2000i128;
        let allocations = client.get_split_allocations(&total_amount);

        assert_eq!(allocations.len(), 4);
        let expected_amounts = [1000, 600, 300, 100];
        let categories = [
            symbol_short!("SPENDING"),
            symbol_short!("SAVINGS"),
            symbol_short!("BILLS"),
            symbol_short!("INSURANCE"),
        ];

        for i in 0..4 {
            let allocation = allocations.get(i).unwrap();
            let idx = i as usize;
            assert_eq!(allocation.amount, expected_amounts[idx]);
            assert_eq!(allocation.category, categories[idx]);
        }
    }
}
