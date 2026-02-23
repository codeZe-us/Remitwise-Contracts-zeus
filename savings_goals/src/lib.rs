#![no_std]
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Map, String,
    Symbol, Vec,
};

// Event topics
const GOAL_CREATED: Symbol = symbol_short!("created");
const FUNDS_ADDED: Symbol = symbol_short!("added");
const GOAL_COMPLETED: Symbol = symbol_short!("completed");

// Event data structures
#[derive(Clone)]
#[contracttype]
pub struct GoalCreatedEvent {
    pub goal_id: u32,
    pub name: String,
    pub target_amount: i128,
    pub target_date: u64,
    pub timestamp: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct FundsAddedEvent {
    pub goal_id: u32,
    pub amount: i128,
    pub new_total: i128,
    pub timestamp: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct GoalCompletedEvent {
    pub goal_id: u32,
    pub name: String,
    pub final_amount: i128,
    pub timestamp: u64,
}

// Storage TTL constants
const INSTANCE_LIFETIME_THRESHOLD: u32 = 17280; // ~1 day
const INSTANCE_BUMP_AMOUNT: u32 = 518400; // ~30 days

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SavingsGoalError {
    ContractPaused = 1,
    FunctionPaused = 2,
    Unauthorized = 3,
    TimeLockedUnpauseNotReached = 4,
    TargetAmountMustBePositive = 5,
    AmountMustBePositive = 6,
    GoalNotFound = 7,
    BatchTooLarge = 8,
    NotOwnerOfAllGoals = 9,
    BatchValidationFailed = 10,
    GoalLocked = 11,
    TimeLocked = 12,
    InsufficientBalance = 13,
    UnsupportedSnapshotVersion = 14,
    SnapshotChecksumMismatch = 15,
    InvalidNonce = 16,
    UnlockDateMustBeInFuture = 17,
    NextDueDateMustBeInFuture = 18,
    ScheduleNotFound = 19,
    ArithmeticError = 20,
}

/// Savings goal data structure with owner tracking for access control
#[contract]
pub struct SavingsGoalContract;

#[contracttype]
#[derive(Clone)]
pub struct SavingsGoal {
    pub id: u32,
    pub owner: Address,
    pub name: String,
    pub target_amount: i128,
    pub current_amount: i128,
    pub target_date: u64,
    pub locked: bool,
    pub unlock_date: Option<u64>,
}

/// Schedule for automatic savings deposits
#[contracttype]
#[derive(Clone)]
pub struct SavingsSchedule {
    pub id: u32,
    pub owner: Address,
    pub goal_id: u32,
    pub amount: i128,
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
pub enum SavingsEvent {
    GoalCreated,
    FundsAdded,
    FundsWithdrawn,
    GoalCompleted,
    GoalLocked,
    GoalUnlocked,
    ScheduleCreated,
    ScheduleExecuted,
    ScheduleMissed,
    ScheduleModified,
    ScheduleCancelled,
}

/// Snapshot for goals export/import (migration). Checksum is numeric for on-chain verification.
#[contracttype]
#[derive(Clone)]
pub struct GoalsExportSnapshot {
    pub version: u32,
    pub checksum: u64,
    pub next_id: u32,
    pub goals: Vec<SavingsGoal>,
}

/// Audit log entry for security and compliance.
#[contracttype]
#[derive(Clone)]
pub struct AuditEntry {
    pub operation: Symbol,
    pub caller: Address,
    pub timestamp: u64,
    pub success: bool,
}

const SNAPSHOT_VERSION: u32 = 1;
const MAX_AUDIT_ENTRIES: u32 = 100;
const CONTRACT_VERSION: u32 = 1;
const MAX_BATCH_SIZE: u32 = 50;

pub mod pause_functions {
    use soroban_sdk::{symbol_short, Symbol};
    pub const CREATE_GOAL: Symbol = symbol_short!("crt_goal");
    pub const ADD_TO_GOAL: Symbol = symbol_short!("add_goal");
    pub const WITHDRAW: Symbol = symbol_short!("withdraw");
    pub const LOCK: Symbol = symbol_short!("lock");
    pub const UNLOCK: Symbol = symbol_short!("unlock");
}

/// Single contribution for batch_add_to_goals
#[contracttype]
#[derive(Clone)]
pub struct ContributionItem {
    pub goal_id: u32,
    pub amount: i128,
}

#[contractimpl]
impl SavingsGoalContract {
    // Storage keys
    const STORAGE_NEXT_ID: Symbol = symbol_short!("NEXT_ID");
    const STORAGE_GOALS: Symbol = symbol_short!("GOALS");

    /// Initialize contract storage
    pub fn init(env: Env) {
        let storage = env.storage().persistent();

        if storage.get::<_, u32>(&Self::STORAGE_NEXT_ID).is_none() {
            storage.set(&Self::STORAGE_NEXT_ID, &1u32);
        }

        if storage
            .get::<_, Map<u32, SavingsGoal>>(&Self::STORAGE_GOALS)
            .is_none()
        {
            storage.set(&Self::STORAGE_GOALS, &Map::<u32, SavingsGoal>::new(&env));
        }
    }

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
    fn require_not_paused(env: &Env, func: Symbol) -> Result<(), SavingsGoalError> {
        if Self::get_global_paused(env) {
            return Err(SavingsGoalError::ContractPaused);
        }
        if Self::is_function_paused(env, func) {
            return Err(SavingsGoalError::FunctionPaused);
        }
        Ok(())
    }

    pub fn set_pause_admin(
        env: Env,
        caller: Address,
        new_admin: Address,
    ) -> Result<(), SavingsGoalError> {
        caller.require_auth();
        let current = Self::get_pause_admin(&env);
        match current {
            None => {
                if caller != new_admin {
                    return Err(SavingsGoalError::Unauthorized);
                }
            }
            Some(admin) if admin != caller => return Err(SavingsGoalError::Unauthorized),
            _ => {}
        }
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSE_ADM"), &new_admin);
        Ok(())
    }
    pub fn pause(env: Env, caller: Address) -> Result<(), SavingsGoalError> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).ok_or(SavingsGoalError::Unauthorized)?;
        if admin != caller {
            return Err(SavingsGoalError::Unauthorized);
        }
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSED"), &true);
        env.events()
            .publish((symbol_short!("savings"), symbol_short!("paused")), ());
        Ok(())
    }
    pub fn unpause(env: Env, caller: Address) -> Result<(), SavingsGoalError> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).ok_or(SavingsGoalError::Unauthorized)?;
        if admin != caller {
            return Err(SavingsGoalError::Unauthorized);
        }
        let unpause_at: Option<u64> = env.storage().instance().get(&symbol_short!("UNP_AT"));
        if let Some(at) = unpause_at {
            if env.ledger().timestamp() < at {
                return Err(SavingsGoalError::TimeLockedUnpauseNotReached);
            }
            env.storage().instance().remove(&symbol_short!("UNP_AT"));
        }
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSED"), &false);
        env.events()
            .publish((symbol_short!("savings"), symbol_short!("unpaused")), ());
        Ok(())
    }
    pub fn pause_function(env: Env, caller: Address, func: Symbol) -> Result<(), SavingsGoalError> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).ok_or(SavingsGoalError::Unauthorized)?;
        if admin != caller {
            return Err(SavingsGoalError::Unauthorized);
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
    pub fn unpause_function(
        env: Env,
        caller: Address,
        func: Symbol,
    ) -> Result<(), SavingsGoalError> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).ok_or(SavingsGoalError::Unauthorized)?;
        if admin != caller {
            return Err(SavingsGoalError::Unauthorized);
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
    ) -> Result<(), SavingsGoalError> {
        caller.require_auth();
        let current = Self::get_upgrade_admin(&env);
        match current {
            None => {
                if caller != new_admin {
                    return Err(SavingsGoalError::Unauthorized);
                }
            }
            Some(adm) if adm != caller => return Err(SavingsGoalError::Unauthorized),
            _ => {}
        }
        env.storage()
            .instance()
            .set(&symbol_short!("UPG_ADM"), &new_admin);
        Ok(())
    }
    pub fn set_version(
        env: Env,
        caller: Address,
        new_version: u32,
    ) -> Result<(), SavingsGoalError> {
        caller.require_auth();
        let admin = Self::get_upgrade_admin(&env).ok_or(SavingsGoalError::Unauthorized)?;
        if admin != caller {
            return Err(SavingsGoalError::Unauthorized);
        }
        let prev = Self::get_version(env.clone());
        env.storage()
            .instance()
            .set(&symbol_short!("VERSION"), &new_version);
        env.events().publish(
            (symbol_short!("savings"), symbol_short!("upgraded")),
            (prev, new_version),
        );
        Ok(())
    }

    /// Create a new savings goal
    ///
    /// # Arguments
    /// * `owner` - Address of the goal owner (must authorize)
    /// * `name` - Name of the goal (e.g., "Education", "Medical")
    /// * `target_amount` - Target amount to save (must be positive)
    /// * `target_date` - Target date as Unix timestamp
    ///
    /// # Returns
    /// The ID of the created goal
    ///
    /// # Panics
    /// - If owner doesn't authorize the transaction
    /// - If target_amount is not positive
    pub fn create_goal(
        env: Env,
        owner: Address,
        name: String,
        target_amount: i128,
        target_date: u64,
    ) -> Result<u32, SavingsGoalError> {
        // Access control: require owner authorization
        owner.require_auth();
        Self::require_not_paused(&env, pause_functions::CREATE_GOAL)?;

        // Input validation
        if target_amount <= 0 {
            Self::append_audit(&env, symbol_short!("create"), &owner, false);
            return Err(SavingsGoalError::TargetAmountMustBePositive);
        }

        // Extend storage TTL
        Self::extend_instance_ttl(&env);

        let mut goals: Map<u32, SavingsGoal> = env
            .storage()
            .instance()
            .get(&symbol_short!("GOALS"))
            .unwrap_or_else(|| Map::new(&env));

        let next_id = env
            .storage()
            .instance()
            .get(&symbol_short!("NEXT_ID"))
            .unwrap_or(0u32)
            + 1;

        let goal = SavingsGoal {
            id: next_id,
            owner: owner.clone(),
            name: name.clone(),
            target_amount,
            current_amount: 0,
            target_date,
            locked: true,
            unlock_date: None,
        };

        goals.set(next_id, goal.clone());
        env.storage()
            .instance()
            .set(&symbol_short!("GOALS"), &goals);
        env.storage()
            .instance()
            .set(&symbol_short!("NEXT_ID"), &next_id);

        // Emit GoalCreated event
        let event = GoalCreatedEvent {
            goal_id: next_id,
            name: goal.name.clone(),
            target_amount,
            target_date,
            timestamp: env.ledger().timestamp(),
        };
        env.events().publish((GOAL_CREATED,), event);
        // Emit event for audit trail
        env.events().publish(
            (symbol_short!("savings"), SavingsEvent::GoalCreated),
            (next_id, owner),
        );

        Ok(next_id)
    }

    /// Add funds to a savings goal
    ///
    /// # Arguments
    /// * `caller` - Address of the caller (must be the goal owner)
    /// * `goal_id` - ID of the goal
    /// * `amount` - Amount to add (must be positive)
    ///
    /// # Returns
    /// Updated current amount
    ///
    /// # Panics
    /// - If caller is not the goal owner
    /// - If goal is not found
    /// - If amount is not positive
    pub fn add_to_goal(
        env: Env,
        caller: Address,
        goal_id: u32,
        amount: i128,
    ) -> Result<i128, SavingsGoalError> {
        // Access control: require caller authorization
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::ADD_TO_GOAL)?;

        // Input validation
        if amount <= 0 {
            Self::append_audit(&env, symbol_short!("add"), &caller, false);
            return Err(SavingsGoalError::AmountMustBePositive);
        }

        // Extend storage TTL
        Self::extend_instance_ttl(&env);

        let mut goals: Map<u32, SavingsGoal> = env
            .storage()
            .instance()
            .get(&symbol_short!("GOALS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut goal = match goals.get(goal_id) {
            Some(g) => g,
            None => {
                Self::append_audit(&env, symbol_short!("add"), &caller, false);
                return Err(SavingsGoalError::GoalNotFound);
            }
        };

        // Access control: verify caller is the owner
        if goal.owner != caller {
            Self::append_audit(&env, symbol_short!("add"), &caller, false);
            return Err(SavingsGoalError::GoalNotFound);
        }

        goal.current_amount = goal
            .current_amount
            .checked_add(amount)
            .ok_or(SavingsGoalError::ArithmeticError)?;
        let new_total = goal.current_amount;
        let was_completed = new_total >= goal.target_amount;
        let previously_completed = (new_total - amount) >= goal.target_amount;

        goals.set(goal_id, goal.clone());
        env.storage()
            .instance()
            .set(&symbol_short!("GOALS"), &goals);

        // Emit FundsAdded event
        let funds_event = FundsAddedEvent {
            goal_id,
            amount,
            new_total,
            timestamp: env.ledger().timestamp(),
        };
        env.events().publish((FUNDS_ADDED,), funds_event);

        // Emit GoalCompleted struct event if it just became completed
        if was_completed && !previously_completed {
            let completed_event = GoalCompletedEvent {
                goal_id,
                name: goal.name.clone(),
                final_amount: new_total,
                timestamp: env.ledger().timestamp(),
            };
            env.events().publish((GOAL_COMPLETED,), completed_event);
        }

        // Emit Audit/Enum Events
        Self::append_audit(&env, symbol_short!("add"), &caller, true);
        env.events().publish(
            (symbol_short!("savings"), SavingsEvent::FundsAdded),
            (goal_id, caller.clone(), amount),
        );

        if was_completed {
            env.events().publish(
                (symbol_short!("savings"), SavingsEvent::GoalCompleted),
                (goal_id, caller),
            );
        }

        Ok(new_total)
    }

    /// Batch add to multiple goals (atomic). Caller must be owner of all goals.
    pub fn batch_add_to_goals(
        env: Env,
        caller: Address,
        contributions: Vec<ContributionItem>,
    ) -> Result<u32, SavingsGoalError> {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::ADD_TO_GOAL)?;
        if contributions.len() as u32 > MAX_BATCH_SIZE {
            return Err(SavingsGoalError::BatchTooLarge);
        }
        let goals_map: Map<u32, SavingsGoal> = env
            .storage()
            .instance()
            .get(&symbol_short!("GOALS"))
            .unwrap_or_else(|| Map::new(&env));
        for item in contributions.iter() {
            if item.amount <= 0 {
                return Err(SavingsGoalError::AmountMustBePositive);
            }
            let goal = goals_map
                .get(item.goal_id)
                .ok_or(SavingsGoalError::GoalNotFound)?;
            if goal.owner != caller {
                return Err(SavingsGoalError::NotOwnerOfAllGoals);
            }
        }
        Self::extend_instance_ttl(&env);
        let mut goals: Map<u32, SavingsGoal> = env
            .storage()
            .instance()
            .get(&symbol_short!("GOALS"))
            .unwrap_or_else(|| Map::new(&env));
        let mut count = 0u32;
        for item in contributions.iter() {
            let mut goal = goals
                .get(item.goal_id)
                .ok_or(SavingsGoalError::GoalNotFound)?;
            if goal.owner != caller {
                return Err(SavingsGoalError::BatchValidationFailed);
            }
            goal.current_amount = goal
                .current_amount
                .checked_add(item.amount)
                .ok_or(SavingsGoalError::ArithmeticError)?;
            let new_total = goal.current_amount;
            let was_completed = new_total >= goal.target_amount;
            let previously_completed = (new_total - item.amount) >= goal.target_amount;
            goals.set(item.goal_id, goal.clone());
            let funds_event = FundsAddedEvent {
                goal_id: item.goal_id,
                amount: item.amount,
                new_total,
                timestamp: env.ledger().timestamp(),
            };
            env.events().publish((FUNDS_ADDED,), funds_event);
            if was_completed && !previously_completed {
                let completed_event = GoalCompletedEvent {
                    goal_id: item.goal_id,
                    name: goal.name.clone(),
                    final_amount: new_total,
                    timestamp: env.ledger().timestamp(),
                };
                env.events().publish((GOAL_COMPLETED,), completed_event);
            }
            env.events().publish(
                (symbol_short!("savings"), SavingsEvent::FundsAdded),
                (item.goal_id, caller.clone(), item.amount),
            );
            if was_completed {
                env.events().publish(
                    (symbol_short!("savings"), SavingsEvent::GoalCompleted),
                    (item.goal_id, caller.clone()),
                );
            }
            count += 1;
        }
        env.storage()
            .instance()
            .set(&symbol_short!("GOALS"), &goals);
        env.events().publish(
            (symbol_short!("savings"), symbol_short!("batch_add")),
            (count, caller),
        );
        Ok(count)
    }

    /// Withdraw funds from a savings goal
    ///
    /// # Arguments
    /// * `caller` - Address of the caller (must be the goal owner)
    /// * `goal_id` - ID of the goal
    /// * `amount` - Amount to withdraw (must be positive and <= current_amount)
    ///
    /// # Returns
    /// Updated current amount
    ///
    /// # Panics
    /// - If caller is not the goal owner
    /// - If goal is not found
    /// - If goal is locked
    /// - If unlock_date is set and not yet reached
    /// - If amount is not positive
    /// - If amount exceeds current balance
    pub fn withdraw_from_goal(
        env: Env,
        caller: Address,
        goal_id: u32,
        amount: i128,
    ) -> Result<i128, SavingsGoalError> {
        // Access control: require caller authorization
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::WITHDRAW)?;

        // Input validation
        if amount <= 0 {
            Self::append_audit(&env, symbol_short!("withdraw"), &caller, false);
            return Err(SavingsGoalError::AmountMustBePositive);
        }

        // Extend storage TTL
        Self::extend_instance_ttl(&env);

        let mut goals: Map<u32, SavingsGoal> = env
            .storage()
            .instance()
            .get(&symbol_short!("GOALS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut goal = match goals.get(goal_id) {
            Some(g) => g,
            None => {
                Self::append_audit(&env, symbol_short!("withdraw"), &caller, false);
                return Err(SavingsGoalError::GoalNotFound);
            }
        };

        // Access control: verify caller is the owner
        if goal.owner != caller {
            Self::append_audit(&env, symbol_short!("withdraw"), &caller, false);
            return Err(SavingsGoalError::Unauthorized);
        }

        // Check if goal is locked
        if goal.locked {
            Self::append_audit(&env, symbol_short!("withdraw"), &caller, false);
            return Err(SavingsGoalError::GoalLocked);
        }

        // Check time-lock
        if let Some(unlock_date) = goal.unlock_date {
            let current_time = env.ledger().timestamp();
            if current_time < unlock_date {
                Self::append_audit(&env, symbol_short!("withdraw"), &caller, false);
                return Err(SavingsGoalError::TimeLocked);
            }
        }

        // Check sufficient balance // NOTE: added check for target vs Amount is not needed
        if amount > goal.current_amount {
            Self::append_audit(&env, symbol_short!("withdraw"), &caller, false);
            return Err(SavingsGoalError::InsufficientBalance);
        }

        goal.current_amount = goal
            .current_amount
            .checked_sub(amount)
            .ok_or(SavingsGoalError::ArithmeticError)?;
        let new_amount = goal.current_amount;

        goals.set(goal_id, goal);
        env.storage()
            .instance()
            .set(&symbol_short!("GOALS"), &goals);

        Self::append_audit(&env, symbol_short!("withdraw"), &caller, true);
        env.events().publish(
            (symbol_short!("savings"), SavingsEvent::FundsWithdrawn),
            (goal_id, caller, amount),
        );

        Ok(new_amount)
    }

    /// Lock a savings goal (prevent withdrawals)
    ///
    /// # Arguments
    /// * `caller` - Address of the caller (must be the goal owner)
    /// * `goal_id` - ID of the goal
    ///
    /// # Panics
    /// - If caller is not the goal owner
    /// - If goal is not found
    pub fn lock_goal(env: Env, caller: Address, goal_id: u32) -> Result<bool, SavingsGoalError> {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::LOCK)?;
        Self::extend_instance_ttl(&env);

        let mut goals: Map<u32, SavingsGoal> = env
            .storage()
            .instance()
            .get(&symbol_short!("GOALS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut goal = match goals.get(goal_id) {
            Some(g) => g,
            None => {
                Self::append_audit(&env, symbol_short!("lock"), &caller, false);
                return Err(SavingsGoalError::GoalNotFound);
            }
        };

        if goal.owner != caller {
            Self::append_audit(&env, symbol_short!("lock"), &caller, false);
            return Err(SavingsGoalError::Unauthorized);
        }

        goal.locked = true;
        goals.set(goal_id, goal);
        env.storage()
            .instance()
            .set(&symbol_short!("GOALS"), &goals);

        Self::append_audit(&env, symbol_short!("lock"), &caller, true);
        env.events().publish(
            (symbol_short!("savings"), SavingsEvent::GoalLocked),
            (goal_id, caller),
        );

        Ok(true)
    }

    /// Unlock a savings goal (allow withdrawals)
    ///
    /// # Arguments
    /// * `caller` - Address of the caller (must be the goal owner)
    /// * `goal_id` - ID of the goal
    ///
    /// # Panics
    /// - If caller is not the goal owner
    /// - If goal is not found
    pub fn unlock_goal(env: Env, caller: Address, goal_id: u32) -> Result<bool, SavingsGoalError> {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::UNLOCK)?;
        Self::extend_instance_ttl(&env);

        let mut goals: Map<u32, SavingsGoal> = env
            .storage()
            .instance()
            .get(&symbol_short!("GOALS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut goal = match goals.get(goal_id) {
            Some(g) => g,
            None => {
                Self::append_audit(&env, symbol_short!("unlock"), &caller, false);
                return Err(SavingsGoalError::GoalNotFound);
            }
        };

        if goal.owner != caller {
            Self::append_audit(&env, symbol_short!("unlock"), &caller, false);
            return Err(SavingsGoalError::Unauthorized);
        }

        goal.locked = false;
        goals.set(goal_id, goal);
        env.storage()
            .instance()
            .set(&symbol_short!("GOALS"), &goals);

        Self::append_audit(&env, symbol_short!("unlock"), &caller, true);
        env.events().publish(
            (symbol_short!("savings"), SavingsEvent::GoalUnlocked),
            (goal_id, caller),
        );

        Ok(true)
    }

    /// Get a savings goal by ID
    ///
    /// # Arguments
    /// * `goal_id` - ID of the goal
    ///
    /// # Returns
    /// SavingsGoal struct or None if not found
    pub fn get_goal(env: Env, goal_id: u32) -> Option<SavingsGoal> {
        let goals: Map<u32, SavingsGoal> = env
            .storage()
            .instance()
            .get(&symbol_short!("GOALS"))
            .unwrap_or_else(|| Map::new(&env));

        goals.get(goal_id)
    }

    /// Get all savings goals for a specific owner
    ///
    /// # Arguments
    /// * `owner` - Address of the goal owner
    ///
    /// # Returns
    /// Vec of all SavingsGoal structs belonging to the owner
    pub fn get_all_goals(env: Env, owner: Address) -> Vec<SavingsGoal> {
        let goals: Map<u32, SavingsGoal> = env
            .storage()
            .instance()
            .get(&symbol_short!("GOALS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        for (_, goal) in goals.iter() {
            if goal.owner == owner {
                result.push_back(goal);
            }
        }
        result
    }

    /// Check if a goal is completed
    pub fn is_goal_completed(env: Env, goal_id: u32) -> bool {
        let storage = env.storage().instance();
        let goals: Map<u32, SavingsGoal> = storage
            .get(&symbol_short!("GOALS"))
            .unwrap_or(Map::new(&env));
        if let Some(goal) = goals.get(goal_id) {
            goal.current_amount >= goal.target_amount
        } else {
            false
        }
    }

    /// Get current nonce for an address (for import_snapshot replay protection).
    pub fn get_nonce(env: Env, address: Address) -> u64 {
        let nonces: Option<Map<Address, u64>> =
            env.storage().instance().get(&symbol_short!("NONCES"));
        nonces
            .as_ref()
            .and_then(|m: &Map<Address, u64>| m.get(address))
            .unwrap_or(0)
    }

    /// Export all goals as snapshot for backup/migration.
    pub fn export_snapshot(env: Env, caller: Address) -> GoalsExportSnapshot {
        caller.require_auth();
        let goals: Map<u32, SavingsGoal> = env
            .storage()
            .instance()
            .get(&symbol_short!("GOALS"))
            .unwrap_or_else(|| Map::new(&env));
        let next_id = env
            .storage()
            .instance()
            .get(&symbol_short!("NEXT_ID"))
            .unwrap_or(0u32);
        let mut list = Vec::new(&env);
        for i in 1..=next_id {
            if let Some(g) = goals.get(i) {
                list.push_back(g);
            }
        }
        let checksum = Self::compute_goals_checksum(SNAPSHOT_VERSION, next_id, &list);
        GoalsExportSnapshot {
            version: SNAPSHOT_VERSION,
            checksum,
            next_id,
            goals: list,
        }
    }

    /// Import snapshot (full restore). Validates version and checksum. Requires nonce for replay protection.
    pub fn import_snapshot(
        env: Env,
        caller: Address,
        nonce: u64,
        snapshot: GoalsExportSnapshot,
    ) -> Result<bool, SavingsGoalError> {
        caller.require_auth();
        Self::require_nonce(&env, &caller, nonce)?;

        if snapshot.version != SNAPSHOT_VERSION {
            Self::append_audit(&env, symbol_short!("import"), &caller, false);
            return Err(SavingsGoalError::UnsupportedSnapshotVersion);
        }
        let expected =
            Self::compute_goals_checksum(snapshot.version, snapshot.next_id, &snapshot.goals);
        if snapshot.checksum != expected {
            Self::append_audit(&env, symbol_short!("import"), &caller, false);
            return Err(SavingsGoalError::SnapshotChecksumMismatch);
        }

        Self::extend_instance_ttl(&env);
        let mut goals: Map<u32, SavingsGoal> = Map::new(&env);
        for g in snapshot.goals.iter() {
            goals.set(g.id, g);
        }
        env.storage()
            .instance()
            .set(&symbol_short!("GOALS"), &goals);
        env.storage()
            .instance()
            .set(&symbol_short!("NEXT_ID"), &snapshot.next_id);

        Self::increment_nonce(&env, &caller);
        Self::append_audit(&env, symbol_short!("import"), &caller, true);
        Ok(true)
    }

    /// Return recent audit log entries.
    pub fn get_audit_log(env: Env, from_index: u32, limit: u32) -> Vec<AuditEntry> {
        let log: Option<Vec<AuditEntry>> = env.storage().instance().get(&symbol_short!("AUDIT"));
        let log = log.unwrap_or_else(|| Vec::new(&env));
        let len = log.len();
        let cap = MAX_AUDIT_ENTRIES.min(limit);
        let mut out = Vec::new(&env);
        if from_index >= len {
            return out;
        }
        let end = (from_index + cap).min(len);
        for i in from_index..end {
            if let Some(entry) = log.get(i) {
                out.push_back(entry);
            }
        }
        out
    }

    fn require_nonce(env: &Env, address: &Address, expected: u64) -> Result<(), SavingsGoalError> {
        let current = Self::get_nonce(env.clone(), address.clone());
        if expected != current {
            return Err(SavingsGoalError::InvalidNonce);
        }
        Ok(())
    }

    fn increment_nonce(env: &Env, address: &Address) {
        let current = Self::get_nonce(env.clone(), address.clone());
        let next = current.checked_add(1).expect("nonce overflow");
        let mut nonces: Map<Address, u64> = env
            .storage()
            .instance()
            .get(&symbol_short!("NONCES"))
            .unwrap_or_else(|| Map::new(env));
        nonces.set(address.clone(), next);
        env.storage()
            .instance()
            .set(&symbol_short!("NONCES"), &nonces);
    }

    fn compute_goals_checksum(version: u32, next_id: u32, goals: &Vec<SavingsGoal>) -> u64 {
        let mut c = version as u64 + next_id as u64;
        for i in 0..goals.len() {
            if let Some(g) = goals.get(i) {
                c = c
                    .wrapping_add(g.id as u64)
                    .wrapping_add(g.target_amount as u64)
                    .wrapping_add(g.current_amount as u64);
            }
        }
        c.wrapping_mul(31)
    }

    fn append_audit(env: &Env, operation: Symbol, caller: &Address, success: bool) {
        let timestamp = env.ledger().timestamp();
        let mut log: Vec<AuditEntry> = env
            .storage()
            .instance()
            .get(&symbol_short!("AUDIT"))
            .unwrap_or_else(|| Vec::new(env));
        if log.len() >= MAX_AUDIT_ENTRIES {
            let mut new_log = Vec::new(env);
            for i in 1..log.len() {
                if let Some(entry) = log.get(i) {
                    new_log.push_back(entry);
                }
            }
            log = new_log;
        }
        log.push_back(AuditEntry {
            operation,
            caller: caller.clone(),
            timestamp,
            success,
        });
        env.storage().instance().set(&symbol_short!("AUDIT"), &log);
    }

    /// Extend the TTL of instance storage
    fn extend_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }

    /// Set time-lock on a goal
    pub fn set_time_lock(
        env: Env,
        caller: Address,
        goal_id: u32,
        unlock_date: u64,
    ) -> Result<bool, SavingsGoalError> {
        caller.require_auth();
        Self::extend_instance_ttl(&env);

        let mut goals: Map<u32, SavingsGoal> = env
            .storage()
            .instance()
            .get(&symbol_short!("GOALS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut goal = match goals.get(goal_id) {
            Some(g) => g,
            None => {
                Self::append_audit(&env, symbol_short!("timelock"), &caller, false);
                return Err(SavingsGoalError::GoalNotFound);
            }
        };

        if goal.owner != caller {
            Self::append_audit(&env, symbol_short!("timelock"), &caller, false);
            return Err(SavingsGoalError::Unauthorized);
        }

        let current_time = env.ledger().timestamp();
        if unlock_date <= current_time {
            Self::append_audit(&env, symbol_short!("timelock"), &caller, false);
            return Err(SavingsGoalError::UnlockDateMustBeInFuture);
        }

        goal.unlock_date = Some(unlock_date);
        goals.set(goal_id, goal);
        env.storage()
            .instance()
            .set(&symbol_short!("GOALS"), &goals);

        Self::append_audit(&env, symbol_short!("timelock"), &caller, true);
        Ok(true)
    }

    /// Create a schedule for automatic savings deposits
    pub fn create_savings_schedule(
        env: Env,
        owner: Address,
        goal_id: u32,
        amount: i128,
        next_due: u64,
        interval: u64,
    ) -> Result<u32, SavingsGoalError> {
        owner.require_auth();

        if amount <= 0 {
            return Err(SavingsGoalError::AmountMustBePositive);
        }

        let goals: Map<u32, SavingsGoal> = env
            .storage()
            .instance()
            .get(&symbol_short!("GOALS"))
            .unwrap_or_else(|| Map::new(&env));

        let goal = goals.get(goal_id).ok_or(SavingsGoalError::GoalNotFound)?;

        if goal.owner != owner {
            return Err(SavingsGoalError::Unauthorized);
        }

        let current_time = env.ledger().timestamp();
        if next_due <= current_time {
            return Err(SavingsGoalError::NextDueDateMustBeInFuture);
        }

        Self::extend_instance_ttl(&env);

        let mut schedules: Map<u32, SavingsSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("SAV_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        let next_schedule_id = env
            .storage()
            .instance()
            .get(&symbol_short!("NEXT_SSCH"))
            .unwrap_or(0u32)
            + 1;

        let schedule = SavingsSchedule {
            id: next_schedule_id,
            owner: owner.clone(),
            goal_id,
            amount,
            next_due,
            interval,
            recurring: interval > 0,
            active: true,
            created_at: current_time,
            last_executed: None,
            missed_count: 0,
        };

        schedules.set(next_schedule_id, schedule);
        env.storage()
            .instance()
            .set(&symbol_short!("SAV_SCH"), &schedules);
        env.storage()
            .instance()
            .set(&symbol_short!("NEXT_SSCH"), &next_schedule_id);

        env.events().publish(
            (symbol_short!("savings"), SavingsEvent::ScheduleCreated),
            (next_schedule_id, owner),
        );

        Ok(next_schedule_id)
    }

    /// Modify a savings schedule
    pub fn modify_savings_schedule(
        env: Env,
        caller: Address,
        schedule_id: u32,
        amount: i128,
        next_due: u64,
        interval: u64,
    ) -> Result<bool, SavingsGoalError> {
        caller.require_auth();

        if amount <= 0 {
            return Err(SavingsGoalError::AmountMustBePositive);
        }

        let current_time = env.ledger().timestamp();
        if next_due <= current_time {
            return Err(SavingsGoalError::NextDueDateMustBeInFuture);
        }

        Self::extend_instance_ttl(&env);

        let mut schedules: Map<u32, SavingsSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("SAV_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        let mut schedule = schedules
            .get(schedule_id)
            .ok_or(SavingsGoalError::ScheduleNotFound)?;

        if schedule.owner != caller {
            return Err(SavingsGoalError::Unauthorized);
        }

        schedule.amount = amount;
        schedule.next_due = next_due;
        schedule.interval = interval;
        schedule.recurring = interval > 0;

        schedules.set(schedule_id, schedule);
        env.storage()
            .instance()
            .set(&symbol_short!("SAV_SCH"), &schedules);

        env.events().publish(
            (symbol_short!("savings"), SavingsEvent::ScheduleModified),
            (schedule_id, caller),
        );

        Ok(true)
    }

    /// Cancel a savings schedule
    pub fn cancel_savings_schedule(
        env: Env,
        caller: Address,
        schedule_id: u32,
    ) -> Result<bool, SavingsGoalError> {
        caller.require_auth();

        Self::extend_instance_ttl(&env);

        let mut schedules: Map<u32, SavingsSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("SAV_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        let mut schedule = schedules
            .get(schedule_id)
            .ok_or(SavingsGoalError::ScheduleNotFound)?;

        if schedule.owner != caller {
            return Err(SavingsGoalError::Unauthorized);
        }

        schedule.active = false;

        schedules.set(schedule_id, schedule);
        env.storage()
            .instance()
            .set(&symbol_short!("SAV_SCH"), &schedules);

        env.events().publish(
            (symbol_short!("savings"), SavingsEvent::ScheduleCancelled),
            (schedule_id, caller),
        );

        Ok(true)
    }

    /// Execute due savings schedules (public, callable by anyone - keeper pattern)
    pub fn execute_due_savings_schedules(env: Env) -> Result<Vec<u32>, SavingsGoalError> {
        Self::extend_instance_ttl(&env);

        let current_time = env.ledger().timestamp();
        let mut executed = Vec::new(&env);

        let mut schedules: Map<u32, SavingsSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("SAV_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        let mut goals: Map<u32, SavingsGoal> = env
            .storage()
            .instance()
            .get(&symbol_short!("GOALS"))
            .unwrap_or_else(|| Map::new(&env));

        for (schedule_id, mut schedule) in schedules.iter() {
            if !schedule.active || schedule.next_due > current_time {
                continue;
            }

            if let Some(mut goal) = goals.get(schedule.goal_id) {
                goal.current_amount = goal
                    .current_amount
                    .checked_add(schedule.amount)
                    .ok_or(SavingsGoalError::ArithmeticError)?;

                let is_completed = goal.current_amount >= goal.target_amount;
                goals.set(schedule.goal_id, goal.clone());

                env.events().publish(
                    (symbol_short!("savings"), SavingsEvent::FundsAdded),
                    (schedule.goal_id, goal.owner.clone(), schedule.amount),
                );

                if is_completed {
                    env.events().publish(
                        (symbol_short!("savings"), SavingsEvent::GoalCompleted),
                        (schedule.goal_id, goal.owner),
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
                        (symbol_short!("savings"), SavingsEvent::ScheduleMissed),
                        (schedule_id, missed),
                    );
                }
            } else {
                schedule.active = false;
            }

            schedules.set(schedule_id, schedule);
            executed.push_back(schedule_id);

            env.events().publish(
                (symbol_short!("savings"), SavingsEvent::ScheduleExecuted),
                schedule_id,
            );
        }

        env.storage()
            .instance()
            .set(&symbol_short!("SAV_SCH"), &schedules);
        env.storage()
            .instance()
            .set(&symbol_short!("GOALS"), &goals);

        Ok(executed)
    }

    /// Get all savings schedules for an owner
    pub fn get_savings_schedules(env: Env, owner: Address) -> Vec<SavingsSchedule> {
        let schedules: Map<u32, SavingsSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("SAV_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        for (_, schedule) in schedules.iter() {
            if schedule.owner == owner {
                result.push_back(schedule);
            }
        }
        result
    }

    /// Get a specific savings schedule
    pub fn get_savings_schedule(env: Env, schedule_id: u32) -> Option<SavingsSchedule> {
        let schedules: Map<u32, SavingsSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("SAV_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        schedules.get(schedule_id)
    }
}

#[cfg(test)]
mod test;
