# RemitWise Smart Contracts

Stellar Soroban smart contracts for the RemitWise remittance platform.

## Overview

This workspace contains the core smart contracts that power RemitWise's post-remittance financial planning features:

- **remittance_split**: Automatically splits remittances into spending, savings, bills, and insurance
- **savings_goals**: Goal-based savings with target dates and locked funds
- **bill_payments**: Automated bill payment tracking and scheduling
- **insurance**: Micro-insurance policy management and premium payments
- **family_wallet**: Family governance, multisig approvals, and emergency transfer controls

## Prerequisites

- Rust (latest stable version)
- Stellar CLI (soroban-cli)
- Cargo

## Installation

```bash
# Install Soroban CLI
cargo install --locked --version 21.0.0 soroban-cli

# Build all contracts
cargo build --release --target wasm32-unknown-unknown
```

## Documentation

- [Family Wallet Design (as implemented)](docs/family-wallet-design.md)
- [Frontend Integration Notes](docs/frontend-integration.md)
- [Storage Layout Reference](STORAGE_LAYOUT.md)

## Contracts

### Remittance Split

Handles automatic allocation of remittance funds into different categories.

**Key Functions:**

- `initialize_split`: Set percentage allocation (spending, savings, bills, insurance)
- `get_split`: Get current split configuration
- `calculate_split`: Calculate actual amounts from total remittance

**Events:**
- `SplitInitializedEvent`: Emitted when split configuration is initialized
  - `spending_percent`, `savings_percent`, `bills_percent`, `insurance_percent`, `timestamp`
- `SplitCalculatedEvent`: Emitted when split amounts are calculated
  - `total_amount`, `spending_amount`, `savings_amount`, `bills_amount`, `insurance_amount`, `timestamp`

### Savings Goals

Manages goal-based savings with target dates.

**Key Functions:**

- `create_goal`: Create a new savings goal (education, medical, etc.)
- `add_to_goal`: Add funds to a goal
- `get_goal`: Get goal details
- `is_goal_completed`: Check if goal target is reached
- `archive_completed_goals`: Archive completed goals to reduce storage
- `get_archived_goals`: Query archived goals
- `restore_goal`: Restore archived goal to active storage
- `cleanup_old_archives`: Permanently delete old archives
- `get_storage_stats`: Get storage usage statistics

**Events:**
- `GoalCreatedEvent`: Emitted when a new savings goal is created
  - `goal_id`, `name`, `target_amount`, `target_date`, `timestamp`
- `FundsAddedEvent`: Emitted when funds are added to a goal
  - `goal_id`, `amount`, `new_total`, `timestamp`
- `GoalCompletedEvent`: Emitted when a goal reaches its target amount
  - `goal_id`, `name`, `final_amount`, `timestamp`

### Bill Payments

Tracks and manages bill payments with recurring support.

**Key Functions:**

- `create_bill`: Create a new bill (electricity, school fees, etc.)
- `pay_bill`: Mark a bill as paid and create next recurring bill if applicable
- `get_unpaid_bills`: Get all unpaid bills
- `get_total_unpaid`: Get total amount of unpaid bills
- `archive_paid_bills`: Archive paid bills to reduce storage
- `get_archived_bills`: Query archived bills
- `restore_bill`: Restore archived bill to active storage
- `bulk_cleanup_bills`: Permanently delete old archives
- `get_storage_stats`: Get storage usage statistics

**Events:**
- `BillCreatedEvent`: Emitted when a new bill is created
  - `bill_id`, `name`, `amount`, `due_date`, `recurring`, `timestamp`
- `BillPaidEvent`: Emitted when a bill is marked as paid
  - `bill_id`, `name`, `amount`, `timestamp`
- `RecurringBillCreatedEvent`: Emitted when a recurring bill generates the next bill
  - `bill_id`, `parent_bill_id`, `name`, `amount`, `due_date`, `timestamp`

### Insurance

Manages micro-insurance policies and premium payments.

**Key Functions:**

- `create_policy`: Create a new insurance policy
- `pay_premium`: Pay monthly premium
- `get_active_policies`: Get all active policies
- `get_total_monthly_premium`: Calculate total monthly premium cost
- `deactivate_policy`: Deactivate an insurance policy

**Events:**
- `PolicyCreatedEvent`: Emitted when a new insurance policy is created
  - `policy_id`, `name`, `coverage_type`, `monthly_premium`, `coverage_amount`, `timestamp`
- `PremiumPaidEvent`: Emitted when a premium is paid
  - `policy_id`, `name`, `amount`, `next_payment_date`, `timestamp`
- `PolicyDeactivatedEvent`: Emitted when a policy is deactivated
  - `policy_id`, `name`, `timestamp`

### Family Wallet

Manages family roles, spending controls, multisig approvals, and emergency transfer policies.

**Key Functions:**

- `init`: Initialize owner, members, default multisig configs, and emergency settings
- `add_member` / `update_spending_limit`: Manage role assignments and per-member spending limits
- `configure_multisig`, `propose_transaction`, `sign_transaction`: Configure and execute multisig-gated actions
- `withdraw`: Execute direct or multisig withdrawal depending on configured threshold
- `configure_emergency`, `set_emergency_mode`, `propose_emergency_transfer`: Configure and run emergency transfer paths
- `pause`, `unpause`, `set_pause_admin`: Operational control switches
- `archive_old_transactions`, `cleanup_expired_pending`, `get_storage_stats`: Storage maintenance and observability

For full design details, see [docs/family-wallet-design.md](docs/family-wallet-design.md).

## Events

All contracts emit events for important state changes, enabling real-time tracking and frontend integration. Events follow Soroban best practices and include:

- **Relevant IDs**: All events include the ID of the entity being acted upon
- **Amounts**: Financial events include transaction amounts
- **Timestamps**: All events include the ledger timestamp for accurate tracking
- **Context Data**: Additional contextual information (names, dates, etc.)

### Event Topics

Each contract uses short symbol topics for efficient event identification:
- **Remittance Split**: `init`, `calc`
- **Savings Goals**: `created`, `added`, `completed`
- **Bill Payments**: `created`, `paid`, `recurring`
- **Insurance**: `created`, `paid`, `deactive`
- **Family Wallet**: `added/member`, `updated/limit`, `emerg/*`, `wallet/*`

### Querying Events

Events can be queried from the Stellar network using the Soroban SDK or via the Horizon API for frontend integration. Each event structure is exported and can be decoded using the contract's schema.

## Testing

Run tests for all contracts:

```bash
cargo test
```

Run tests for a specific contract:

```bash
cd remittance_split
cargo test
```

### USDC remittance split checks (local & CI)

- `cargo test -p remittance_split` exercises the USDC distribution logic with a mocked Stellar Asset Contract (`env.register_stellar_asset_contract_v2`) and built-in auth mocking.
- The suite covers minting the payer account, splitting across spending/savings/bills/insurance, and asserting balances along with the new allocation metadata helper.
- The same command is intended for CI so it runs without manual setup; re-run locally whenever split logic changes or new USDC paths are added.

## Gas Benchmarks

See `docs/gas-optimization.md` for methodology, before/after results, and assumptions.

### Running Locally

Run all benchmarks and generate a JSON report:

```bash
./scripts/run_gas_benchmarks.sh
```

This creates `gas_results.json` with CPU and memory costs for all contract operations.

Or run individual contract benchmarks:

```bash
RUST_TEST_THREADS=1 cargo test -p bill_payments --test gas_bench -- --nocapture
RUST_TEST_THREADS=1 cargo test -p savings_goals --test gas_bench -- --nocapture
RUST_TEST_THREADS=1 cargo test -p insurance --test gas_bench -- --nocapture
RUST_TEST_THREADS=1 cargo test -p family_wallet --test gas_bench -- --nocapture
RUST_TEST_THREADS=1 cargo test -p remittance_split --test gas_bench -- --nocapture
```

### Regression Detection

Compare current results against a baseline:

```bash
# Save current results as baseline
cp gas_results.json baseline.json

# Make changes, then compare
./scripts/run_gas_benchmarks.sh
./scripts/compare_gas_results.sh baseline.json gas_results.json 10
```

The comparison script fails if CPU or memory increases by more than the threshold (default 10%).

### CI Integration

Gas benchmarks run automatically in CI on every push and pull request. Results are uploaded as artifacts and retained for 30 days.

To view results:
1. Go to Actions tab in GitHub
2. Select a workflow run
3. Download the `gas-benchmarks` artifact
4. View `gas_results.json` for metrics

## Seed data for local development

After deploying contracts to a local or test network, you can seed them with realistic example data (goals, bills, policies, remittance split, and optionally family members) using deterministic values for stable IDs.

1. **Deploy** the contracts (see [Deployment](DEPLOYMENT.md)) and note the contract IDs.
2. **Create a signer identity** (if needed) and fund it on the target network:
   ```bash
   soroban keys generate deployer
   # Fund the deployer address (e.g. via friendbot on testnet)
   ```
3. **Run the seed script** with your contract IDs and network:
   ```bash
   export REMITTANCE_SPLIT_ID=<id>
   export SAVINGS_GOALS_ID=<id>
   export BILL_PAYMENTS_ID=<id>
   export INSURANCE_ID=<id>
   export NETWORK=testnet
   export SOURCE=deployer
   ./scripts/seed_local.sh
   ```
   Or pass IDs as arguments: `./scripts/seed_local.sh $REMITTANCE_SPLIT_ID $SAVINGS_GOALS_ID $BILL_PAYMENTS_ID $INSURANCE_ID`

   Optional: set `SEED_FAMILY=1` and `FAMILY_WALLET_ID=<id>` to initialize the family wallet with the owner. Add members later via `add_member` or extend the script.

The script requires the Soroban CLI (or Stellar CLI). It creates three savings goals, three bills, two insurance policies, and one remittance split (50/30/15/5). Re-running on the same deployment will create additional entities; for a clean slate, deploy fresh contracts.

## Deployment

See the [Deployment Guide](DEPLOYMENT.md) for comprehensive deployment instructions.

Quick deploy to testnet:

```bash
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/remittance_split.wasm \
  --source <your-key> \
  --network testnet
```

## Development

This is a basic MVP implementation. Future enhancements:

- Integration with Stellar Asset Contract (USDC)
- Cross-contract calls for automated allocation
- Multi-signature support for family wallets
- Emergency mode with priority processing

## License

MIT
