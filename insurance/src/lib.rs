#![no_std]
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Map, String,
    Symbol, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum InsuranceError {
    PolicyNotFound = 1,
    Unauthorized = 2,
    InvalidAmount = 3,
    PolicyInactive = 4,
    ContractPaused = 5,
    FunctionPaused = 6,
    InvalidTimestamp = 7,
    BatchTooLarge = 8,
}

// Event topics
const POLICY_CREATED: Symbol = symbol_short!("created");
const PREMIUM_PAID: Symbol = symbol_short!("paid");
const POLICY_DEACTIVATED: Symbol = symbol_short!("deactive");

// Event data structures
#[derive(Clone)]
#[contracttype]
pub struct PolicyCreatedEvent {
    pub policy_id: u32,
    pub name: String,
    pub coverage_type: String,
    pub monthly_premium: i128,
    pub coverage_amount: i128,
    pub timestamp: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct PremiumPaidEvent {
    pub policy_id: u32,
    pub name: String,
    pub amount: i128,
    pub next_payment_date: u64,
    pub timestamp: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct PolicyDeactivatedEvent {
    pub policy_id: u32,
    pub name: String,
    pub timestamp: u64,
}

// Storage TTL constants
const INSTANCE_LIFETIME_THRESHOLD: u32 = 17280; // ~1 day
const INSTANCE_BUMP_AMOUNT: u32 = 518400; // ~30 days

const CONTRACT_VERSION: u32 = 1;

pub mod pause_functions {
    use soroban_sdk::{symbol_short, Symbol};
    pub const CREATE_POLICY: Symbol = symbol_short!("crt_pol");
    pub const PAY_PREMIUM: Symbol = symbol_short!("pay_prem");
    pub const DEACTIVATE: Symbol = symbol_short!("deact");
    pub const CREATE_SCHED: Symbol = symbol_short!("crt_sch");
    pub const MODIFY_SCHED: Symbol = symbol_short!("mod_sch");
    pub const CANCEL_SCHED: Symbol = symbol_short!("can_sch");
}

/// Insurance policy data structure with owner tracking for access control
#[derive(Clone)]
#[contracttype]
pub struct InsurancePolicy {
    pub id: u32,
    pub owner: Address,
    pub name: String,
    pub coverage_type: String,
    pub monthly_premium: i128,
    pub coverage_amount: i128,
    pub active: bool,
    pub next_payment_date: u64,
    pub schedule_id: Option<u32>,
}

/// Schedule for automatic premium payments
#[contracttype]
#[derive(Clone)]
pub struct PremiumSchedule {
    pub id: u32,
    pub owner: Address,
    pub policy_id: u32,
    pub next_due: u64,
    pub interval: u64,
    pub recurring: bool,
    pub active: bool,
    pub created_at: u64,
    pub last_executed: Option<u64>,
    pub missed_count: u32,
}

/// Events emitted by the contract for audit trail
#[contracttype]
#[derive(Clone)]
pub enum InsuranceEvent {
    PolicyCreated,
    PremiumPaid,
    PolicyDeactivated,
    ScheduleCreated,
    ScheduleExecuted,
    ScheduleMissed,
    ScheduleModified,
    ScheduleCancelled,
}

#[contract]
pub struct Insurance;

#[contractimpl]
impl Insurance {
    fn get_pause_admin(env: &Env) -> Option<Address> {
        env.storage().instance().get(&symbol_short!("PAUSE_ADM"))
    }
    fn get_global_paused(env: &Env) -> bool {
        env.storage()
            .instance()
            .get(&symbol_short!("PAUSED"))
            .unwrap_or(false)
    }
    fn is_function_paused(env: &Env, func: Symbol) -> bool {
        env.storage()
            .instance()
            .get::<_, Map<Symbol, bool>>(&symbol_short!("PAUSED_FN"))
            .unwrap_or_else(|| Map::new(env))
            .get(func)
            .unwrap_or(false)
    }
    fn require_not_paused(env: &Env, func: Symbol) -> Result<(), InsuranceError> {
        if Self::get_global_paused(env) {
            return Err(InsuranceError::ContractPaused);
        }
        if Self::is_function_paused(env, func) {
            return Err(InsuranceError::FunctionPaused);
        }
        Ok(())
    }

    pub fn set_pause_admin(
        env: Env,
        caller: Address,
        new_admin: Address,
    ) -> Result<(), InsuranceError> {
        caller.require_auth();
        let current = Self::get_pause_admin(&env);
        match current {
            None => {
                if caller != new_admin {
                    return Err(InsuranceError::Unauthorized);
                }
            }
            Some(admin) if admin != caller => return Err(InsuranceError::Unauthorized),
            _ => {}
        }
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSE_ADM"), &new_admin);
        Ok(())
    }
    pub fn pause(env: Env, caller: Address) -> Result<(), InsuranceError> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).ok_or(InsuranceError::Unauthorized)?;
        if admin != caller {
            return Err(InsuranceError::Unauthorized);
        }
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSED"), &true);
        env.events()
            .publish((symbol_short!("insure"), symbol_short!("paused")), ());
        Ok(())
    }
    pub fn unpause(env: Env, caller: Address) -> Result<(), InsuranceError> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).ok_or(InsuranceError::Unauthorized)?;
        if admin != caller {
            return Err(InsuranceError::Unauthorized);
        }
        let unpause_at: Option<u64> = env.storage().instance().get(&symbol_short!("UNP_AT"));
        if let Some(at) = unpause_at {
            if env.ledger().timestamp() < at {
                panic!("Time-locked unpause not yet reached");
            }
            env.storage().instance().remove(&symbol_short!("UNP_AT"));
        }
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSED"), &false);
        env.events()
            .publish((symbol_short!("insure"), symbol_short!("unpaused")), ());
        Ok(())
    }
    pub fn pause_function(env: Env, caller: Address, func: Symbol) -> Result<(), InsuranceError> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).expect("No pause admin set");
        if admin != caller {
            return Err(InsuranceError::Unauthorized);
        }
        let mut m: Map<Symbol, bool> = env
            .storage()
            .instance()
            .get(&symbol_short!("PAUSED_FN"))
            .unwrap_or_else(|| Map::new(&env));
        m.set(func, true);
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSED_FN"), &m);
        Ok(())
    }
    pub fn unpause_function(env: Env, caller: Address, func: Symbol) -> Result<(), InsuranceError> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).expect("No pause admin set");
        if admin != caller {
            return Err(InsuranceError::Unauthorized);
        }
        let mut m: Map<Symbol, bool> = env
            .storage()
            .instance()
            .get(&symbol_short!("PAUSED_FN"))
            .unwrap_or_else(|| Map::new(&env));
        m.set(func, false);
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSED_FN"), &m);
        Ok(())
    }
    pub fn emergency_pause_all(env: Env, caller: Address) {
        let _ = Self::pause(env.clone(), caller.clone());
        for func in [
            pause_functions::CREATE_POLICY,
            pause_functions::PAY_PREMIUM,
            pause_functions::DEACTIVATE,
            pause_functions::CREATE_SCHED,
            pause_functions::MODIFY_SCHED,
            pause_functions::CANCEL_SCHED,
        ] {
            let _ = Self::pause_function(env.clone(), caller.clone(), func);
        }
    }
    pub fn is_paused(env: Env) -> bool {
        Self::get_global_paused(&env)
    }
    pub fn get_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&symbol_short!("VERSION"))
            .unwrap_or(CONTRACT_VERSION)
    }
    fn get_upgrade_admin(env: &Env) -> Option<Address> {
        env.storage().instance().get(&symbol_short!("UPG_ADM"))
    }
    pub fn set_upgrade_admin(
        env: Env,
        caller: Address,
        new_admin: Address,
    ) -> Result<(), InsuranceError> {
        caller.require_auth();
        let current = Self::get_upgrade_admin(&env);
        match current {
            None => {
                if caller != new_admin {
                    return Err(InsuranceError::Unauthorized);
                }
            }
            Some(adm) if adm != caller => return Err(InsuranceError::Unauthorized),
            _ => {}
        }
        env.storage()
            .instance()
            .set(&symbol_short!("UPG_ADM"), &new_admin);
        Ok(())
    }
    pub fn set_version(env: Env, caller: Address, new_version: u32) -> Result<(), InsuranceError> {
        caller.require_auth();
        let admin = Self::get_upgrade_admin(&env).expect("No upgrade admin set");
        if admin != caller {
            return Err(InsuranceError::Unauthorized);
        }
        let prev = Self::get_version(env.clone());
        env.storage()
            .instance()
            .set(&symbol_short!("VERSION"), &new_version);
        env.events().publish(
            (symbol_short!("insure"), symbol_short!("upgraded")),
            (prev, new_version),
        );
        Ok(())
    }

    /// Create a new insurance policy
    ///
    /// # Arguments
    /// * `owner` - Address of the policy owner (must authorize)
    /// * `name` - Name of the policy
    /// * `coverage_type` - Type of coverage (e.g., "health", "emergency")
    /// * `monthly_premium` - Monthly premium amount (must be positive)
    /// * `coverage_amount` - Total coverage amount (must be positive)
    ///
    /// # Returns
    /// The ID of the created policy
    ///
    /// # Panics
    /// - If owner doesn't authorize the transaction
    /// - If monthly_premium is not positive
    /// - If coverage_amount is not positive
    pub fn create_policy(
        env: Env,
        owner: Address,
        name: String,
        coverage_type: String,
        monthly_premium: i128,
        coverage_amount: i128,
    ) -> Result<u32, InsuranceError> {
        owner.require_auth();
        Self::require_not_paused(&env, pause_functions::CREATE_POLICY)?;

        if monthly_premium <= 0 || coverage_amount <= 0 {
            return Err(InsuranceError::InvalidAmount);
        }

        Self::extend_instance_ttl(&env);

        let mut policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        let next_id = env
            .storage()
            .instance()
            .get(&symbol_short!("NEXT_ID"))
            .unwrap_or(0u32)
            + 1;

        let next_payment_date = env.ledger().timestamp() + (30 * 86400);

        let policy = InsurancePolicy {
            id: next_id,
            owner: owner.clone(),
            name: name.clone(),
            coverage_type: coverage_type.clone(),
            monthly_premium,
            coverage_amount,
            active: true,
            next_payment_date,
            schedule_id: None,
        };

        policies.set(next_id, policy);
        env.storage()
            .instance()
            .set(&symbol_short!("POLICIES"), &policies);
        env.storage()
            .instance()
            .set(&symbol_short!("NEXT_ID"), &next_id);

        env.events().publish(
            (POLICY_CREATED,),
            PolicyCreatedEvent {
                policy_id: next_id,
                name,
                coverage_type,
                monthly_premium,
                coverage_amount,
                timestamp: env.ledger().timestamp(),
            },
        );

        env.events().publish(
            (symbol_short!("insure"), InsuranceEvent::PolicyCreated),
            (next_id, owner),
        );

        Ok(next_id)
    }

    /// Pay monthly premium for a policy
    ///
    /// # Arguments
    /// * `caller` - Address of the caller (must be the policy owner)
    /// * `policy_id` - ID of the policy
    ///
    /// # Returns
    /// True if payment was successful
    ///
    /// # Panics
    /// - If caller is not the policy owner
    /// - If policy is not found
    /// - If policy is not active
    pub fn pay_premium(env: Env, caller: Address, policy_id: u32) -> Result<bool, InsuranceError> {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::PAY_PREMIUM)?;
        Self::extend_instance_ttl(&env);

        let mut policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut policy = policies
            .get(policy_id)
            .ok_or(InsuranceError::PolicyNotFound)?;

        if policy.owner != caller {
            return Err(InsuranceError::Unauthorized);
        }
        if !policy.active {
            return Err(InsuranceError::PolicyInactive);
        }

        policy.next_payment_date = env.ledger().timestamp() + (30 * 86400);
        policies.set(policy_id, policy.clone());
        env.storage()
            .instance()
            .set(&symbol_short!("POLICIES"), &policies);

        env.events().publish(
            (PREMIUM_PAID,),
            PremiumPaidEvent {
                policy_id,
                name: policy.name,
                amount: policy.monthly_premium,
                next_payment_date: policy.next_payment_date,
                timestamp: env.ledger().timestamp(),
            },
        );

        env.events().publish(
            (symbol_short!("insure"), InsuranceEvent::PremiumPaid),
            (policy_id, caller),
        );

        Ok(true)
    }

    /// Batch pay premiums for multiple policies (atomic). Caller must be owner of all.
    pub fn batch_pay_premiums(
        env: Env,
        caller: Address,
        policy_ids: Vec<u32>,
    ) -> Result<u32, InsuranceError> {
        caller.require_auth();
        if policy_ids.len() > 20 {
            return Err(InsuranceError::BatchTooLarge);
        }

        let mut count = 0;
        for id in policy_ids.iter() {
            Self::pay_premium(env.clone(), caller.clone(), id)?;
            count += 1;
        }
        Ok(count)
    }

    /// Get a policy by ID
    ///
    /// # Arguments
    /// * `policy_id` - ID of the policy
    ///
    /// # Returns
    /// InsurancePolicy struct or None if not found
    pub fn get_policy(env: Env, policy_id: u32) -> Option<InsurancePolicy> {
        let policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        policies.get(policy_id)
    }

    /// Get all active policies for a specific owner
    ///
    /// # Arguments
    /// * `owner` - Address of the policy owner
    ///
    /// # Returns
    /// Vec of active InsurancePolicy structs belonging to the owner
    pub fn get_active_policies(env: Env, owner: Address) -> Vec<InsurancePolicy> {
        let policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        for (_, policy) in policies.iter() {
            if policy.active && policy.owner == owner {
                result.push_back(policy);
            }
        }
        result
    }

    /// Get total monthly premium for all active policies of an owner
    ///
    /// # Arguments
    /// * `owner` - Address of the policy owner
    ///
    /// # Returns
    /// Total monthly premium amount for the owner's active policies
    pub fn get_total_monthly_premium(env: Env, owner: Address) -> i128 {
        let mut total = 0i128;
        let policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        for (_, policy) in policies.iter() {
            if policy.active && policy.owner == owner {
                total += policy.monthly_premium;
            }
        }
        total
    }

    /// Deactivate a policy
    ///
    /// # Arguments
    /// * `caller` - Address of the caller (must be the policy owner)
    /// * `policy_id` - ID of the policy
    ///
    /// # Returns
    /// True if deactivation was successful
    ///
    /// # Panics
    /// - If caller is not the policy owner
    /// - If policy is not found
    pub fn deactivate_policy(
        env: Env,
        caller: Address,
        policy_id: u32,
    ) -> Result<bool, InsuranceError> {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::DEACTIVATE)?;

        let mut policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut policy = policies
            .get(policy_id)
            .ok_or(InsuranceError::PolicyNotFound)?;

        if policy.owner != caller {
            return Err(InsuranceError::Unauthorized);
        }

        policy.active = false;
        policies.set(policy_id, policy.clone());
        env.storage()
            .instance()
            .set(&symbol_short!("POLICIES"), &policies);

        env.events().publish(
            (POLICY_DEACTIVATED,),
            PolicyDeactivatedEvent {
                policy_id,
                name: policy.name,
                timestamp: env.ledger().timestamp(),
            },
        );

        env.events().publish(
            (symbol_short!("insure"), InsuranceEvent::PolicyDeactivated),
            (policy_id, caller),
        );

        Ok(true)
    }

    /// Extend the TTL of instance storage
    fn extend_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }

    /// Create a schedule for automatic premium payments
    pub fn create_premium_schedule(
        env: Env,
        owner: Address,
        policy_id: u32,
        next_due: u64,
        interval: u64,
    ) -> Result<u32, InsuranceError> {
        // Changed to Result
        owner.require_auth();
        Self::require_not_paused(&env, pause_functions::CREATE_SCHED)?;

        let mut policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut policy = policies
            .get(policy_id)
            .ok_or(InsuranceError::PolicyNotFound)?;

        if policy.owner != owner {
            return Err(InsuranceError::Unauthorized);
        }

        let current_time = env.ledger().timestamp();
        if next_due <= current_time {
            return Err(InsuranceError::InvalidTimestamp);
        }

        Self::extend_instance_ttl(&env);

        let mut schedules: Map<u32, PremiumSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("PREM_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        let next_schedule_id = env
            .storage()
            .instance()
            .get(&symbol_short!("NEXT_PSCH"))
            .unwrap_or(0u32)
            + 1;

        let schedule = PremiumSchedule {
            id: next_schedule_id,
            owner: owner.clone(),
            policy_id,
            next_due,
            interval,
            recurring: interval > 0,
            active: true,
            created_at: current_time,
            last_executed: None,
            missed_count: 0,
        };

        policy.schedule_id = Some(next_schedule_id);

        schedules.set(next_schedule_id, schedule);
        env.storage()
            .instance()
            .set(&symbol_short!("PREM_SCH"), &schedules);
        env.storage()
            .instance()
            .set(&symbol_short!("NEXT_PSCH"), &next_schedule_id);

        policies.set(policy_id, policy);
        env.storage()
            .instance()
            .set(&symbol_short!("POLICIES"), &policies);

        env.events().publish(
            (symbol_short!("insure"), InsuranceEvent::ScheduleCreated),
            (next_schedule_id, owner),
        );

        Ok(next_schedule_id)
    }

    /// Modify a premium schedule
    pub fn modify_premium_schedule(
        env: Env,
        caller: Address,
        schedule_id: u32,
        next_due: u64,
        interval: u64,
    ) -> Result<bool, InsuranceError> {
        // Changed to Result
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::MODIFY_SCHED)?;

        let current_time = env.ledger().timestamp();
        if next_due <= current_time {
            return Err(InsuranceError::InvalidTimestamp); // Use Err instead of panic
        }

        Self::extend_instance_ttl(&env);

        let mut schedules: Map<u32, PremiumSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("PREM_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        let mut schedule = schedules
            .get(schedule_id)
            .ok_or(InsuranceError::PolicyNotFound)?;

        if schedule.owner != caller {
            return Err(InsuranceError::Unauthorized); // Use Err instead of panic
        }

        schedule.next_due = next_due;
        schedule.interval = interval;
        schedule.recurring = interval > 0;

        schedules.set(schedule_id, schedule);
        env.storage()
            .instance()
            .set(&symbol_short!("PREM_SCH"), &schedules);

        env.events().publish(
            (symbol_short!("insure"), InsuranceEvent::ScheduleModified),
            (schedule_id, caller),
        );

        Ok(true) // Wrap return value in Ok
    }

    /// Cancel a premium schedule
    pub fn cancel_premium_schedule(
        env: Env,
        caller: Address,
        schedule_id: u32,
    ) -> Result<bool, InsuranceError> {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::CANCEL_SCHED)?;

        Self::extend_instance_ttl(&env);

        let mut schedules: Map<u32, PremiumSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("PREM_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        let mut schedule = schedules
            .get(schedule_id)
            .ok_or(InsuranceError::PolicyNotFound)?;

        if schedule.owner != caller {
            return Err(InsuranceError::Unauthorized);
        }

        schedule.active = false;

        schedules.set(schedule_id, schedule);
        env.storage()
            .instance()
            .set(&symbol_short!("PREM_SCH"), &schedules);

        env.events().publish(
            (symbol_short!("insure"), InsuranceEvent::ScheduleCancelled),
            (schedule_id, caller),
        );

        Ok(true)
    }

    /// Execute due premium schedules (public, callable by anyone - keeper pattern)
    pub fn execute_due_premium_schedules(env: Env) -> Vec<u32> {
        Self::extend_instance_ttl(&env);

        let current_time = env.ledger().timestamp();
        let mut executed = Vec::new(&env);

        let mut schedules: Map<u32, PremiumSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("PREM_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        let mut policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        for (schedule_id, mut schedule) in schedules.iter() {
            if !schedule.active || schedule.next_due > current_time {
                continue;
            }

            if let Some(mut policy) = policies.get(schedule.policy_id) {
                if policy.active {
                    policy.next_payment_date = current_time + (30 * 86400);
                    policies.set(schedule.policy_id, policy.clone());

                    env.events().publish(
                        (symbol_short!("insure"), InsuranceEvent::PremiumPaid),
                        (schedule.policy_id, policy.owner),
                    );
                }
            }

            schedule.last_executed = Some(current_time);

            if schedule.recurring && schedule.interval > 0 {
                let mut missed = 0u32;
                let mut next = schedule.next_due + schedule.interval;
                while next <= current_time {
                    missed += 1;
                    next += schedule.interval;
                }
                schedule.missed_count += missed;
                schedule.next_due = next;

                if missed > 0 {
                    env.events().publish(
                        (symbol_short!("insure"), InsuranceEvent::ScheduleMissed),
                        (schedule_id, missed),
                    );
                }
            } else {
                schedule.active = false;
            }

            schedules.set(schedule_id, schedule);
            executed.push_back(schedule_id);

            env.events().publish(
                (symbol_short!("insure"), InsuranceEvent::ScheduleExecuted),
                schedule_id,
            );
        }

        env.storage()
            .instance()
            .set(&symbol_short!("PREM_SCH"), &schedules);
        env.storage()
            .instance()
            .set(&symbol_short!("POLICIES"), &policies);

        executed
    }

    /// Get all premium schedules for an owner
    pub fn get_premium_schedules(env: Env, owner: Address) -> Vec<PremiumSchedule> {
        let schedules: Map<u32, PremiumSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("PREM_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        for (_, schedule) in schedules.iter() {
            if schedule.owner == owner {
                result.push_back(schedule);
            }
        }
        result
    }

    /// Get a specific premium schedule
    pub fn get_premium_schedule(env: Env, schedule_id: u32) -> Option<PremiumSchedule> {
        let schedules: Map<u32, PremiumSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("PREM_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        schedules.get(schedule_id)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::storage::Instance as _;
    use soroban_sdk::testutils::{Address as _, Events, Ledger, LedgerInfo};
    use soroban_sdk::{Env, String};

    fn make_env() -> Env {
        Env::default()
    }

    fn setup_policies(
        env: &Env,
        client: &InsuranceClient,
        owner: &Address,
        count: u32,
    ) -> Vec<u32> {
        let mut ids = Vec::new(env);
        for i in 0..count {
            let id = client.create_policy(
                owner,
                &String::from_str(env, "Policy"),
                &String::from_str(env, "health"),
                &(50i128 * (i as i128 + 1)),
                &(10000i128 * (i as i128 + 1)),
            );
            ids.push_back(id);
        }
        ids
    }

    // --- get_active_policies ---

    #[test]
    fn test_create_policy_invalid_premium() {
        let env = Env::default();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);
        env.mock_all_auths();

        // Use the .try_ version of the function to capture the error result
        let result = client.try_create_policy(
            &owner,
            &String::from_str(&env, "Life"),
            &String::from_str(&env, "Health"),
            &0, // This is invalid
            &10000,
        );

        // Assert that the result matches our custom error code
        assert_eq!(result, Err(Ok(InsuranceError::InvalidAmount)));
    }

    #[test]
    fn test_create_policy_emits_event() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        // Create a policy
        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Health Insurance"),
            &String::from_str(&env, "health"),
            &100,
            &50000,
        );
        assert_eq!(policy_id, 1);

        // Verify event was emitted
        let events = env.events().all();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_pay_premium_emits_event() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        // Create a policy
        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Emergency Coverage"),
            &String::from_str(&env, "emergency"),
            &75,
            &25000,
        );

        env.mock_all_auths();

        // Get events before paying premium
        let events_before = env.events().all().len();

        // Pay premium
        let result = client.pay_premium(&owner, &policy_id);
        assert!(result);

        // Verify PremiumPaid event was emitted (2 new events: topic + enum)
        let events_after = env.events().all().len();
        assert_eq!(events_after - events_before, 2);
    }

    #[test]
    fn test_deactivate_policy_emits_event() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        // Create a policy
        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Life Insurance"),
            &String::from_str(&env, "life"),
            &200,
            &100000,
        );

        env.mock_all_auths();

        // Get events before deactivating
        let events_before = env.events().all().len();

        // Deactivate policy
        let result = client.deactivate_policy(&owner, &policy_id);
        assert!(result);

        // Verify PolicyDeactivated event was emitted (2 new events: topic + enum)
        let events_after = env.events().all().len();
        assert_eq!(events_after - events_before, 2);
    }

    #[test]
    fn test_multiple_policies_emit_separate_events() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        // Create multiple policies
        client.create_policy(
            &owner,
            &String::from_str(&env, "Policy 1"),
            &String::from_str(&env, "health"),
            &100,
            &50000,
        );
        client.create_policy(
            &owner,
            &String::from_str(&env, "Policy 2"),
            &String::from_str(&env, "life"),
            &200,
            &100000,
        );
        client.create_policy(
            &owner,
            &String::from_str(&env, "Policy 3"),
            &String::from_str(&env, "emergency"),
            &75,
            &25000,
        );

        // Should have 6 events (2 per create_policy)
        let events = env.events().all();
        assert_eq!(events.len(), 6);
    }

    #[test]
    fn test_policy_lifecycle_emits_all_events() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        // Create a policy
        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Complete Lifecycle"),
            &String::from_str(&env, "health"),
            &150,
            &75000,
        );

        env.mock_all_auths();

        // Pay premium
        client.pay_premium(&owner, &policy_id);

        // Deactivate
        client.deactivate_policy(&owner, &policy_id);

        // Should have 6 events: 2 Created + 2 PremiumPaid + 2 Deactivated
        let events = env.events().all();
        assert_eq!(events.len(), 6);
    }

    // ====================================================================
    // Storage TTL Extension Tests
    //
    // Verify that instance storage TTL is properly extended on
    // state-changing operations, preventing unexpected data expiration.
    //
    // Contract TTL configuration:
    //   INSTANCE_LIFETIME_THRESHOLD = 17,280 ledgers (~1 day)
    //   INSTANCE_BUMP_AMOUNT        = 518,400 ledgers (~30 days)
    //
    // Operations extending instance TTL:
    //   create_policy, pay_premium, batch_pay_premiums,
    //   deactivate_policy, create_premium_schedule,
    //   modify_premium_schedule, cancel_premium_schedule,
    //   execute_due_premium_schedules
    // ====================================================================

    /// Verify that create_policy extends instance storage TTL.
    #[test]
    fn test_instance_ttl_extended_on_create_policy() {
        let env = Env::default();
        env.mock_all_auths();

        env.ledger().set(LedgerInfo {
            protocol_version: 20,
            sequence_number: 100,
            timestamp: 1000,
            network_id: [0; 32],
            base_reserve: 10,
            min_temp_entry_ttl: 100,
            min_persistent_entry_ttl: 100,
            max_entry_ttl: 700_000,
        });

        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        // create_policy calls extend_instance_ttl
        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Health Insurance"),
            &String::from_str(&env, "health"),
            &100,
            &50000,
        );
        assert_eq!(policy_id, 1);

        // Inspect instance TTL — must be at least INSTANCE_BUMP_AMOUNT
        let ttl = env.as_contract(&contract_id, || env.storage().instance().get_ttl());
        assert!(
            ttl >= 518_400,
            "Instance TTL ({}) must be >= INSTANCE_BUMP_AMOUNT (518,400) after create_policy",
            ttl
        );
    }

    /// Verify that pay_premium refreshes instance TTL after ledger advancement.
    ///
    /// extend_ttl(threshold, extend_to) only extends when TTL <= threshold.
    /// We advance the ledger far enough for TTL to drop below 17,280.
    #[test]
    fn test_instance_ttl_refreshed_on_pay_premium() {
        let env = Env::default();
        env.mock_all_auths();

        env.ledger().set(LedgerInfo {
            protocol_version: 20,
            sequence_number: 100,
            timestamp: 1000,
            network_id: [0; 32],
            base_reserve: 10,
            min_temp_entry_ttl: 100,
            min_persistent_entry_ttl: 100,
            max_entry_ttl: 700_000,
        });

        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        client.create_policy(
            &owner,
            &String::from_str(&env, "Life Insurance"),
            &String::from_str(&env, "life"),
            &200,
            &100000,
        );

        // Advance ledger so TTL drops below threshold (17,280)
        // After create_policy: live_until = 518,500. At seq 510,000: TTL = 8,500
        env.ledger().set(LedgerInfo {
            protocol_version: 20,
            sequence_number: 510_000,
            timestamp: 500_000,
            network_id: [0; 32],
            base_reserve: 10,
            min_temp_entry_ttl: 100,
            min_persistent_entry_ttl: 100,
            max_entry_ttl: 700_000,
        });

        // pay_premium calls extend_instance_ttl → re-extends TTL to 518,400
        client.pay_premium(&owner, &1);

        let ttl = env.as_contract(&contract_id, || env.storage().instance().get_ttl());
        assert!(
            ttl >= 518_400,
            "Instance TTL ({}) must be >= 518,400 after pay_premium",
            ttl
        );
    }

    /// Verify data persists across repeated operations spanning multiple
    /// ledger advancements, proving TTL is continuously renewed.
    #[test]
    fn test_policy_data_persists_across_ledger_advancements() {
        let env = Env::default();
        env.mock_all_auths();

        env.ledger().set(LedgerInfo {
            protocol_version: 20,
            sequence_number: 100,
            timestamp: 1000,
            network_id: [0; 32],
            base_reserve: 10,
            min_temp_entry_ttl: 100,
            min_persistent_entry_ttl: 100,
            max_entry_ttl: 700_000,
        });

        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        // Phase 1: Create policy at seq 100. live_until = 518,500
        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Auto Insurance"),
            &String::from_str(&env, "auto"),
            &150,
            &75000,
        );

        // Phase 2: Advance to seq 510,000 (TTL = 8,500 < 17,280)
        env.ledger().set(LedgerInfo {
            protocol_version: 20,
            sequence_number: 510_000,
            timestamp: 510_000,
            network_id: [0; 32],
            base_reserve: 10,
            min_temp_entry_ttl: 100,
            min_persistent_entry_ttl: 100,
            max_entry_ttl: 700_000,
        });

        client.pay_premium(&owner, &policy_id);

        // Phase 3: Advance to seq 1,020,000 (TTL = 8,400 < 17,280)
        env.ledger().set(LedgerInfo {
            protocol_version: 20,
            sequence_number: 1_020_000,
            timestamp: 1_020_000,
            network_id: [0; 32],
            base_reserve: 10,
            min_temp_entry_ttl: 100,
            min_persistent_entry_ttl: 100,
            max_entry_ttl: 700_000,
        });

        let policy_id2 = client.create_policy(
            &owner,
            &String::from_str(&env, "Travel Insurance"),
            &String::from_str(&env, "travel"),
            &50,
            &20000,
        );

        // All policies should be accessible
        let p1 = client.get_policy(&policy_id);
        assert!(
            p1.is_some(),
            "First policy must persist across ledger advancements"
        );
        assert_eq!(p1.unwrap().monthly_premium, 150);

        let p2 = client.get_policy(&policy_id2);
        assert!(p2.is_some(), "Second policy must persist");

        // TTL should be fully refreshed
        let ttl = env.as_contract(&contract_id, || env.storage().instance().get_ttl());
        assert!(
            ttl >= 518_400,
            "Instance TTL ({}) must remain >= 518,400 after repeated operations",
            ttl
        );
    }

    /// Verify that deactivate_policy extends instance TTL.
    #[test]
    fn test_instance_ttl_extended_on_deactivate_policy() {
        let env = Env::default();
        env.mock_all_auths();

        env.ledger().set(LedgerInfo {
            protocol_version: 20,
            sequence_number: 100,
            timestamp: 1000,
            network_id: [0; 32],
            base_reserve: 10,
            min_temp_entry_ttl: 100,
            min_persistent_entry_ttl: 100,
            max_entry_ttl: 700_000,
        });

        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Dental"),
            &String::from_str(&env, "dental"),
            &75,
            &25000,
        );

        // Advance ledger past threshold
        env.ledger().set(LedgerInfo {
            protocol_version: 20,
            sequence_number: 510_000,
            timestamp: 510_000,
            network_id: [0; 32],
            base_reserve: 10,
            min_temp_entry_ttl: 100,
            min_persistent_entry_ttl: 100,
            max_entry_ttl: 700_000,
        });

        // deactivate_policy calls extend_instance_ttl
        client.deactivate_policy(&owner, &policy_id);

        let ttl = env.as_contract(&contract_id, || env.storage().instance().get_ttl());
        assert!(
            ttl >= 518_400,
            "Instance TTL ({}) must be >= 518,400 after deactivate_policy",
            ttl
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // Test: pay_premium after deactivate_policy (#104)
    // ──────────────────────────────────────────────────────────────────

    /// After deactivating a policy, `pay_premium` must panic with
    /// "Policy is not active". The policy must remain inactive.
    #[test]
    #[should_panic(expected = "Policy is not active")]
    fn test_pay_premium_after_deactivate() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        // 1. Create a policy
        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Health Plan"),
            &String::from_str(&env, "health"),
            &150,
            &50000,
        );

        // Sanity: policy should be active after creation
        let policy_before = client.get_policy(&policy_id).unwrap();
        assert!(policy_before.active);

        // 2. Deactivate the policy
        let deactivated = client.deactivate_policy(&owner, &policy_id);
        assert!(deactivated);

        // Confirm it is now inactive
        let policy_after_deactivate = client.get_policy(&policy_id).unwrap();
        assert!(!policy_after_deactivate.active);

        // 3. Attempt to pay premium — must panic
        client.pay_premium(&owner, &policy_id);
    }

    // ============================================================================
    // Time-Based Payment Date Tests
    // ============================================================================
    // These tests verify that next_payment_date is correctly calculated when
    // policies are created and when premiums are paid, using explicit ledger
    // time control to ensure billing cycles are accurate.

    const PAYMENT_PERIOD_SECONDS: u64 = 30 * 86400; // 30 days in seconds = 2,592,000

    fn set_time(env: &Env, timestamp: u64) {
        let proto = env.ledger().protocol_version();
        env.ledger().set(soroban_sdk::testutils::LedgerInfo {
            protocol_version: proto,
            sequence_number: 1,
            timestamp,
            network_id: [0; 32],
            base_reserve: 10,
            min_temp_entry_ttl: 1,
            min_persistent_entry_ttl: 1,
            max_entry_ttl: 100000,
        });
    }

    #[test]
    fn test_initial_payment_date_calculation() {
        let env = Env::default();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        env.mock_all_auths();

        // Set explicit creation time
        let creation_time = 1_000_000u64;
        set_time(&env, creation_time);

        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Test Policy"),
            &String::from_str(&env, "Health"),
            &100,
            &10000,
        );

        let policy = client.get_policy(&policy_id).unwrap();

        // Verify next_payment_date is exactly creation_time + 30 days
        let expected_next_payment = creation_time + PAYMENT_PERIOD_SECONDS;
        assert_eq!(
            policy.next_payment_date, expected_next_payment,
            "Initial next_payment_date should be creation_time + 30 days"
        );

        // Verify the constant is correct (30 days = 2,592,000 seconds)
        assert_eq!(PAYMENT_PERIOD_SECONDS, 2_592_000);
    }

    #[test]
    fn test_payment_date_update_after_single_payment() {
        let env = Env::default();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        env.mock_all_auths();

        // T0: Create policy at time 1,000,000
        let t0 = 1_000_000u64;
        set_time(&env, t0);

        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Test Policy"),
            &String::from_str(&env, "Health"),
            &100,
            &10000,
        );

        let initial_policy = client.get_policy(&policy_id).unwrap();
        let initial_next_payment = initial_policy.next_payment_date;

        // Verify initial next_payment_date is T0 + 30 days
        assert_eq!(initial_next_payment, t0 + PAYMENT_PERIOD_SECONDS);

        // T1: Advance time by 15 days and pay premium
        let t1 = t0 + (15 * 86400); // 15 days later
        set_time(&env, t1);

        let success = client.pay_premium(&owner, &policy_id);
        assert!(success);

        let updated_policy = client.get_policy(&policy_id).unwrap();

        // Verify next_payment_date is T1 + 30 days (not T0 + 30 days)
        let expected_next_payment = t1 + PAYMENT_PERIOD_SECONDS;
        assert_eq!(
            updated_policy.next_payment_date, expected_next_payment,
            "next_payment_date should be calculated from payment time (T1), not creation time (T0)"
        );

        // Verify it's different from the initial next_payment_date
        assert_ne!(
            updated_policy.next_payment_date, initial_next_payment,
            "next_payment_date should have been updated"
        );
    }

    #[test]
    fn test_sequential_payment_date_updates() {
        let env = Env::default();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        env.mock_all_auths();

        // T0: Create policy
        let t0 = 1_000_000u64;
        set_time(&env, t0);

        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Test Policy"),
            &String::from_str(&env, "Health"),
            &100,
            &10000,
        );

        let policy_after_creation = client.get_policy(&policy_id).unwrap();
        assert_eq!(
            policy_after_creation.next_payment_date,
            t0 + PAYMENT_PERIOD_SECONDS,
            "Initial next_payment_date should be T0 + 30 days"
        );

        // T1: First payment at 20 days after creation
        let t1 = t0 + (20 * 86400);
        set_time(&env, t1);

        client.pay_premium(&owner, &policy_id);

        let policy_after_first_payment = client.get_policy(&policy_id).unwrap();
        let expected_after_first = t1 + PAYMENT_PERIOD_SECONDS;
        assert_eq!(
            policy_after_first_payment.next_payment_date, expected_after_first,
            "After first payment, next_payment_date should be T1 + 30 days"
        );

        // T2: Second payment at 25 days after first payment
        let t2 = t1 + (25 * 86400);
        set_time(&env, t2);

        client.pay_premium(&owner, &policy_id);

        let policy_after_second_payment = client.get_policy(&policy_id).unwrap();
        let expected_after_second = t2 + PAYMENT_PERIOD_SECONDS;
        assert_eq!(
            policy_after_second_payment.next_payment_date, expected_after_second,
            "After second payment, next_payment_date should be T2 + 30 days"
        );

        // Verify each payment independently calculated from current timestamp
        assert_ne!(
            policy_after_first_payment.next_payment_date,
            policy_after_second_payment.next_payment_date,
            "Each payment should update next_payment_date independently"
        );
    }

    #[test]
    fn test_payment_period_constant_accuracy() {
        // Verify the payment period constant is exactly 30 days in seconds
        const SECONDS_PER_DAY: u64 = 86400;
        const DAYS_IN_PERIOD: u64 = 30;

        assert_eq!(
            PAYMENT_PERIOD_SECONDS,
            DAYS_IN_PERIOD * SECONDS_PER_DAY,
            "Payment period should be exactly 30 days"
        );

        assert_eq!(
            PAYMENT_PERIOD_SECONDS, 2_592_000,
            "Payment period should be 2,592,000 seconds"
        );
    }

    #[test]
    fn test_time_advancement_mechanism() {
        let env = Env::default();

        // Test that set_time correctly updates the ledger timestamp
        let timestamp1 = 1_000_000u64;
        set_time(&env, timestamp1);
        assert_eq!(env.ledger().timestamp(), timestamp1);

        // Test advancing time
        let timestamp2 = 2_000_000u64;
        set_time(&env, timestamp2);
        assert_eq!(env.ledger().timestamp(), timestamp2);

        // Verify time can be set to any value
        let timestamp3 = 5_000_000u64;
        set_time(&env, timestamp3);
        assert_eq!(env.ledger().timestamp(), timestamp3);
    }

    #[test]
    fn test_payment_date_with_early_payment() {
        let env = Env::default();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        env.mock_all_auths();

        // Create policy at T0
        let t0 = 1_000_000u64;
        set_time(&env, t0);

        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Test Policy"),
            &String::from_str(&env, "Health"),
            &100,
            &10000,
        );

        // Pay premium early (only 5 days after creation, before the 30-day due date)
        let t_early = t0 + (5 * 86400);
        set_time(&env, t_early);

        client.pay_premium(&owner, &policy_id);

        let policy = client.get_policy(&policy_id).unwrap();

        // next_payment_date should be 30 days from the early payment time
        let expected = t_early + PAYMENT_PERIOD_SECONDS;
        assert_eq!(
            policy.next_payment_date, expected,
            "Early payment should set next_payment_date to 30 days from payment time"
        );
    }

    #[test]
    fn test_payment_date_with_late_payment() {
        let env = Env::default();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        env.mock_all_auths();

        // Create policy at T0
        let t0 = 1_000_000u64;
        set_time(&env, t0);

        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Test Policy"),
            &String::from_str(&env, "Health"),
            &100,
            &10000,
        );

        let initial_policy = client.get_policy(&policy_id).unwrap();
        let initial_due = initial_policy.next_payment_date;

        // Pay premium late (35 days after creation, 5 days past due)
        let t_late = t0 + (35 * 86400);
        set_time(&env, t_late);

        client.pay_premium(&owner, &policy_id);

        let policy = client.get_policy(&policy_id).unwrap();

        // next_payment_date should be 30 days from the late payment time
        let expected = t_late + PAYMENT_PERIOD_SECONDS;
        assert_eq!(
            policy.next_payment_date, expected,
            "Late payment should set next_payment_date to 30 days from payment time"
        );

        // Verify it's later than the original due date
        assert!(
            policy.next_payment_date > initial_due,
            "Late payment should push next_payment_date beyond original due date"
        );
    }

    #[test]
    fn test_multiple_policies_independent_payment_dates() {
        let env = Env::default();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        env.mock_all_auths();

        // Create first policy at T0
        let t0 = 1_000_000u64;
        set_time(&env, t0);

        let policy_id1 = client.create_policy(
            &owner,
            &String::from_str(&env, "Policy 1"),
            &String::from_str(&env, "Health"),
            &100,
            &10000,
        );

        // Create second policy at T1 (10 days later)
        let t1 = t0 + (10 * 86400);
        set_time(&env, t1);

        let policy_id2 = client.create_policy(
            &owner,
            &String::from_str(&env, "Policy 2"),
            &String::from_str(&env, "Life"),
            &200,
            &20000,
        );

        let policy1 = client.get_policy(&policy_id1).unwrap();
        let policy2 = client.get_policy(&policy_id2).unwrap();

        // Verify each policy has independent next_payment_date
        assert_eq!(policy1.next_payment_date, t0 + PAYMENT_PERIOD_SECONDS);
        assert_eq!(policy2.next_payment_date, t1 + PAYMENT_PERIOD_SECONDS);
        assert_ne!(policy1.next_payment_date, policy2.next_payment_date);

        // Pay premium for policy1 at T2
        let t2 = t0 + (15 * 86400);
        set_time(&env, t2);
        client.pay_premium(&owner, &policy_id1);

        let policy1_updated = client.get_policy(&policy_id1).unwrap();
        let policy2_unchanged = client.get_policy(&policy_id2).unwrap();

        // Verify policy1 was updated but policy2 was not
        assert_eq!(
            policy1_updated.next_payment_date,
            t2 + PAYMENT_PERIOD_SECONDS
        );
        assert_eq!(
            policy2_unchanged.next_payment_date,
            t1 + PAYMENT_PERIOD_SECONDS
        );
    }
}
