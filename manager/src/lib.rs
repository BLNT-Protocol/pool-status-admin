//! Admin for Blend pools with two roles, built on the OpenZeppelin Stellar
//! Contracts ownable module.
//!
//! * The owner (meant to be a multisig) holds every pool admin power: it
//!   accepts and hands on the admin role, updates the pool config, queues
//!   and cancels reserve changes, sets emissions, sets the status, and
//!   replaces the operator.
//! * The operator (meant to be a hot key, e.g. a cron job) can only set the
//!   status of the pools this contract administers.
//!
//! A stolen operator key can only open, ice or freeze the pools; the owner
//! replaces the operator.  There is no ownership transfer and no renounce:
//! a multisig owner rotates its own signers, and the owner moves a pool to a
//! different admin with `propose_admin`.  The contract cannot be upgraded.
//!
//! The status-only contract in the repository root is unchanged by this
//! crate.
#![no_std]

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype,
    panic_with_error, Address, Env, Vec,
};
use stellar_access::ownable;
use stellar_macros::only_owner;

/// Extend the instance TTL to ~120 days whenever it falls under ~30 days.
const LEDGERS_PER_DAY: u32 = 17_280;
const TTL_THRESHOLD: u32 = 30 * LEDGERS_PER_DAY;
const TTL_EXTEND_TO: u32 = 120 * LEDGERS_PER_DAY;

/// A reserve's configuration, as Blend v2 and BLNT v3 pools define it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReserveConfig {
    pub index: u32,
    pub decimals: u32,
    pub c_factor: u32,
    pub l_factor: u32,
    pub util: u32,
    pub max_util: u32,
    pub r_base: u32,
    pub r_one: u32,
    pub r_two: u32,
    pub r_three: u32,
    pub reactivity: u32,
    pub supply_cap: i128,
    pub enabled: bool,
}

/// A reserve's emissions share, as Blend v2 and BLNT v3 pools define it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReserveEmissionMetadata {
    pub res_index: u32,
    pub res_type: u32,
    pub share: u64,
}

/// The Blend pool admin functions this contract calls.
#[contractclient(name = "PoolClient")]
pub trait Pool {
    fn propose_admin(e: Env, new_admin: Address);
    fn accept_admin(e: Env);
    fn update_pool(e: Env, backstop_take_rate: u32, max_positions: u32, min_collateral: i128);
    fn queue_set_reserve(e: Env, asset: Address, metadata: ReserveConfig);
    fn cancel_set_reserve(e: Env, asset: Address);
    fn set_status(e: Env, pool_status: u32);
    fn set_emissions_config(e: Env, res_emission_metadata: Vec<ReserveEmissionMetadata>);
}

#[contracttype]
#[derive(Clone)]
enum Key {
    Operator,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ManagerError {
    /// `caller` is neither the owner nor the operator.
    NotOwnerOrOperator = 1,
}

#[contractevent(topics = ["set_operator"], data_format = "single-value")]
struct SetOperator {
    #[topic]
    old: Address,
    new: Address,
}

#[contract]
pub struct PoolStatusManager;

fn bump(e: &Env) {
    e.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn load_operator(e: &Env) -> Address {
    e.storage().instance().get(&Key::Operator).unwrap()
}

#[contractimpl]
impl PoolStatusManager {
    /// Sets the owner and the operator.
    ///
    /// # Arguments
    ///
    /// * `owner` - The account with every admin power.  Cannot be changed.
    /// * `operator` - The account that can only set pool status.
    pub fn __constructor(e: &Env, owner: Address, operator: Address) {
        ownable::set_owner(e, &owner);
        e.storage().instance().set(&Key::Operator, &operator);
        bump(e);
    }

    // ------------------------------------------------------------------
    // Operator or owner

    /// Sets the status of `pool`.  `caller` must be the owner or the
    /// operator, and must authorize the call.
    ///
    /// # Arguments
    ///
    /// * `caller` - The owner or the operator.
    /// * `pool` - A pool this contract is the admin of.
    /// * `pool_status` - `0` admin active, `2` admin on-ice, `3` on-ice
    ///   under permissionless status updates, `4` admin frozen.
    ///
    /// # Errors
    ///
    /// * [`ManagerError::NotOwnerOrOperator`] - `caller` is neither.
    /// * refer to the pool's `set_status` errors.
    pub fn set_status(e: &Env, caller: Address, pool: Address, pool_status: u32) {
        bump(e);
        if Some(caller.clone()) != ownable::get_owner(e) && caller != load_operator(e) {
            panic_with_error!(e, ManagerError::NotOwnerOrOperator);
        }
        caller.require_auth();
        PoolClient::new(e, &pool).set_status(&pool_status);
    }

    // ------------------------------------------------------------------
    // Owner only

    /// Replaces the operator.  Owner only.
    #[only_owner]
    pub fn set_operator(e: &Env, operator: Address) {
        bump(e);
        let old = load_operator(e);
        e.storage().instance().set(&Key::Operator, &operator);
        SetOperator { old, new: operator }.publish(e);
    }

    /// Accepts the admin role on `pool`, once the pool's admin has proposed
    /// this contract.  Owner only.
    #[only_owner]
    pub fn accept_admin(e: &Env, pool: Address) {
        bump(e);
        PoolClient::new(e, &pool).accept_admin();
    }

    /// Proposes `new_admin` as the admin of `pool`; `new_admin` must then
    /// accept.  Owner only.
    #[only_owner]
    pub fn propose_admin(e: &Env, pool: Address, new_admin: Address) {
        bump(e);
        PoolClient::new(e, &pool).propose_admin(&new_admin);
    }

    /// Updates the backstop take rate, max positions and min collateral of
    /// `pool`.  Owner only.
    #[only_owner]
    pub fn update_pool(
        e: &Env,
        pool: Address,
        backstop_take_rate: u32,
        max_positions: u32,
        min_collateral: i128,
    ) {
        bump(e);
        PoolClient::new(e, &pool).update_pool(&backstop_take_rate, &max_positions, &min_collateral);
    }

    /// Queues a reserve change on `pool` (applied by anyone with
    /// `set_reserve` once the pool's timelock passes).  Owner only.
    #[only_owner]
    pub fn queue_set_reserve(e: &Env, pool: Address, asset: Address, metadata: ReserveConfig) {
        bump(e);
        PoolClient::new(e, &pool).queue_set_reserve(&asset, &metadata);
    }

    /// Cancels a queued reserve change on `pool`.  Owner only.
    #[only_owner]
    pub fn cancel_set_reserve(e: &Env, pool: Address, asset: Address) {
        bump(e);
        PoolClient::new(e, &pool).cancel_set_reserve(&asset);
    }

    /// Sets the reserve emissions split of `pool`.  Owner only.
    #[only_owner]
    pub fn set_emissions_config(
        e: &Env,
        pool: Address,
        res_emission_metadata: Vec<ReserveEmissionMetadata>,
    ) {
        bump(e);
        PoolClient::new(e, &pool).set_emissions_config(&res_emission_metadata);
    }

    // ------------------------------------------------------------------
    // Views

    pub fn get_owner(e: &Env) -> Option<Address> {
        ownable::get_owner(e)
    }

    pub fn get_operator(e: &Env) -> Address {
        load_operator(e)
    }
}

#[cfg(test)]
mod test;
