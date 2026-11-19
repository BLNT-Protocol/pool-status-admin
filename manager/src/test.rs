extern crate std;

use soroban_sdk::{
    testutils::{Address as _, ContractFunctionSet as _, Events as _, MockAuth, MockAuthInvoke},
    vec,
    xdr::{ScErrorCode, ScErrorType},
    Address, Env, Error, IntoVal, InvokeError, Val, Vec,
};

use crate::{PoolStatusManager, PoolStatusManagerClient, ReserveConfig, ReserveEmissionMetadata};

/// The admin entrypoints of a Blend v2 / BLNT v3 pool.  Each admin call
/// requires the admin's auth and records what it was given.
mod pool {
    use crate::{ReserveConfig, ReserveEmissionMetadata};
    use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Vec};

    #[contracttype]
    #[derive(Clone)]
    pub enum Key {
        Admin,
        ProposedAdmin,
        Status,
        Config,
        Queued(Address),
        Emissions,
    }

    #[contract]
    pub struct Pool;

    #[contractimpl]
    impl Pool {
        pub fn __constructor(e: &Env, admin: Address) {
            e.storage().instance().set(&Key::Admin, &admin);
            e.storage().instance().set(&Key::Status, &6u32);
        }

        pub fn propose_admin(e: &Env, new_admin: Address) {
            Self::admin(e).require_auth();
            e.storage().temporary().set(&Key::ProposedAdmin, &new_admin);
        }

        pub fn accept_admin(e: &Env) {
            let proposed: Address = e.storage().temporary().get(&Key::ProposedAdmin).unwrap();
            proposed.require_auth();
            e.storage().instance().set(&Key::Admin, &proposed);
        }

        pub fn update_pool(
            e: &Env,
            backstop_take_rate: u32,
            max_positions: u32,
            min_collateral: i128,
        ) {
            Self::admin(e).require_auth();
            e.storage().instance().set(
                &Key::Config,
                &(backstop_take_rate, max_positions, min_collateral),
            );
        }

        pub fn queue_set_reserve(e: &Env, asset: Address, metadata: ReserveConfig) {
            Self::admin(e).require_auth();
            e.storage().instance().set(&Key::Queued(asset), &metadata);
        }

        pub fn cancel_set_reserve(e: &Env, asset: Address) {
            Self::admin(e).require_auth();
            e.storage().instance().remove(&Key::Queued(asset));
        }

        pub fn set_status(e: &Env, pool_status: u32) {
            Self::admin(e).require_auth();
            e.storage().instance().set(&Key::Status, &pool_status);
        }

        pub fn set_emissions_config(e: &Env, res_emission_metadata: Vec<ReserveEmissionMetadata>) {
            Self::admin(e).require_auth();
            e.storage()
                .instance()
                .set(&Key::Emissions, &res_emission_metadata);
        }

        pub fn admin(e: &Env) -> Address {
            e.storage().instance().get(&Key::Admin).unwrap()
        }

        pub fn status(e: &Env) -> u32 {
            e.storage().instance().get(&Key::Status).unwrap()
        }

        pub fn config(e: &Env) -> Option<(u32, u32, i128)> {
            e.storage().instance().get(&Key::Config)
        }

        pub fn queued(e: &Env, asset: Address) -> Option<ReserveConfig> {
            e.storage().instance().get(&Key::Queued(asset))
        }

        pub fn emissions(e: &Env) -> Option<Vec<ReserveEmissionMetadata>> {
            e.storage().instance().get(&Key::Emissions)
        }
    }
}

use pool::{Pool, PoolClient};

type Failure = Result<Error, InvokeError>;

const UNAUTHORIZED: Failure = Ok(Error::from_type_and_code(
    ScErrorType::Context,
    ScErrorCode::InvalidAction,
));
const NOT_OWNER_OR_OPERATOR: Failure = Ok(Error::from_contract_error(1));

const OWNER_FNS: [&str; 7] = [
    "set_operator",
    "accept_admin",
    "propose_admin",
    "update_pool",
    "queue_set_reserve",
    "cancel_set_reserve",
    "set_emissions_config",
];

fn reserve_config() -> ReserveConfig {
    ReserveConfig {
        index: 0,
        decimals: 7,
        c_factor: 7_000_000,
        l_factor: 7_000_000,
        util: 5_000_000,
        max_util: 7_000_000,
        r_base: 100_000,
        r_one: 400_000,
        r_two: 3_000_000,
        r_three: 50_000_000,
        reactivity: 20,
        supply_cap: 1_500_000_000,
        enabled: true,
    }
}

/// A manager and a pool still run by its original admin.
struct Fixture<'a> {
    e: Env,
    manager: PoolStatusManagerClient<'a>,
    pool: PoolClient<'a>,
    owner: Address,
    operator: Address,
}

impl<'a> Fixture<'a> {
    fn new() -> Self {
        let e = Env::default();
        let owner = Address::generate(&e);
        let operator = Address::generate(&e);
        let id = e.register(PoolStatusManager, (owner.clone(), operator.clone()));
        let pool = Self::new_pool(&e);
        Fixture {
            manager: PoolStatusManagerClient::new(&e, &id),
            pool,
            owner,
            operator,
            e,
        }
    }

    /// A fixture whose manager is already the admin of `pool`.
    fn adopted() -> Self {
        let f = Self::new();
        f.propose(&f.pool);
        f.try_accept(&f.owner, &f.pool).unwrap();
        f
    }

    fn new_pool(e: &Env) -> PoolClient<'a> {
        let id = e.register(Pool, (Address::generate(e),));
        PoolClient::new(e, &id)
    }

    /// Authorizes `signer` for exactly one call to the manager and nothing
    /// beneath it.
    fn sign(&self, signer: &Address, fn_name: &str, args: Vec<Val>) {
        self.e.mock_auths(&[MockAuth {
            address: signer,
            invoke: &MockAuthInvoke {
                contract: &self.manager.address,
                fn_name,
                args,
                sub_invokes: &[],
            },
        }]);
    }

    /// The pool's current admin proposes the manager.
    fn propose(&self, pool: &PoolClient) {
        self.e.mock_auths(&[MockAuth {
            address: &pool.admin(),
            invoke: &MockAuthInvoke {
                contract: &pool.address,
                fn_name: "propose_admin",
                args: (&self.manager.address,).into_val(&self.e),
                sub_invokes: &[],
            },
        }]);
        pool.propose_admin(&self.manager.address);
    }

    fn try_accept(&self, signer: &Address, pool: &PoolClient) -> Result<(), Failure> {
        self.sign(signer, "accept_admin", (&pool.address,).into_val(&self.e));
        self.manager
            .try_accept_admin(&pool.address)
            .map(Result::unwrap)
    }

    /// `signer` signs a set_status call that names `caller`.
    fn try_set_status_as(
        &self,
        signer: &Address,
        caller: &Address,
        pool: &PoolClient,
        status: u32,
    ) -> Result<(), Failure> {
        self.sign(
            signer,
            "set_status",
            (caller, &pool.address, status).into_val(&self.e),
        );
        self.manager
            .try_set_status(caller, &pool.address, &status)
            .map(Result::unwrap)
    }

    fn try_set_status(
        &self,
        signer: &Address,
        pool: &PoolClient,
        status: u32,
    ) -> Result<(), Failure> {
        self.try_set_status_as(signer, signer, pool, status)
    }

    /// Calls each owner-only function once, signed by `signer`.  Returns
    /// the result of each call, in OWNER_FNS order.
    fn try_owner_fns(&self, signer: &Address) -> std::vec::Vec<Result<(), Failure>> {
        let e = &self.e;
        let p = &self.pool.address;
        let asset = Address::generate(e);
        let other = Address::generate(e);
        let config = reserve_config();
        let emissions = vec![
            e,
            ReserveEmissionMetadata {
                res_index: 0,
                res_type: 1,
                share: 1_0000000,
            },
        ];
        let m = &self.manager;
        std::vec![
            {
                self.sign(signer, "set_operator", (&other,).into_val(e));
                m.try_set_operator(&other).map(Result::unwrap)
            },
            {
                self.sign(signer, "accept_admin", (p,).into_val(e));
                m.try_accept_admin(p).map(Result::unwrap)
            },
            {
                self.sign(signer, "propose_admin", (p, &other).into_val(e));
                m.try_propose_admin(p, &other).map(Result::unwrap)
            },
            {
                self.sign(
                    signer,
                    "update_pool",
                    (p, 1_000_000u32, 4u32, 50_000_000i128).into_val(e),
                );
                m.try_update_pool(p, &1_000_000, &4, &50_000_000)
                    .map(Result::unwrap)
            },
            {
                self.sign(
                    signer,
                    "queue_set_reserve",
                    (p, &asset, config.clone()).into_val(e),
                );
                m.try_queue_set_reserve(p, &asset, &config)
                    .map(Result::unwrap)
            },
            {
                self.sign(signer, "cancel_set_reserve", (p, &asset).into_val(e));
                m.try_cancel_set_reserve(p, &asset).map(Result::unwrap)
            },
            {
                self.sign(
                    signer,
                    "set_emissions_config",
                    (p, emissions.clone()).into_val(e),
                );
                m.try_set_emissions_config(p, &emissions)
                    .map(Result::unwrap)
            },
        ]
    }
}

#[test]
fn constructor_sets_owner_and_operator() {
    let f = Fixture::new();

    assert_eq!(f.manager.get_owner(), Some(f.owner.clone()));
    assert_eq!(f.manager.get_operator(), f.operator);
}

#[test]
fn owner_accepts_admin_and_sets_status() {
    let f = Fixture::new();

    f.propose(&f.pool);
    assert_eq!(f.try_accept(&f.owner, &f.pool), Ok(()));
    assert_eq!(f.pool.admin(), f.manager.address);

    for status in [4, 2, 3, 0] {
        assert_eq!(f.try_set_status(&f.owner, &f.pool, status), Ok(()));
        assert_eq!(f.pool.status(), status);
    }
}

#[test]
fn operator_sets_status() {
    let f = Fixture::adopted();

    for status in [4, 0, 2, 3, 4] {
        assert_eq!(f.try_set_status(&f.operator, &f.pool, status), Ok(()));
        assert_eq!(f.pool.status(), status);
    }
}

#[test]
fn other_caller_cannot_set_status() {
    let f = Fixture::adopted();
    let other = Address::generate(&f.e);

    assert_eq!(
        f.try_set_status(&other, &f.pool, 0),
        Err(NOT_OWNER_OR_OPERATOR)
    );
    assert_eq!(f.pool.status(), 6);
}

#[test]
fn naming_the_operator_needs_the_operators_signature() {
    let f = Fixture::adopted();
    let other = Address::generate(&f.e);

    assert_eq!(
        f.try_set_status_as(&other, &f.operator, &f.pool, 0),
        Err(UNAUTHORIZED)
    );
    assert_eq!(
        f.try_set_status_as(&other, &f.owner, &f.pool, 0),
        Err(UNAUTHORIZED)
    );
    assert_eq!(f.pool.status(), 6);
}

#[test]
fn set_status_needs_the_admin_role() {
    let f = Fixture::new();

    assert_eq!(f.try_set_status(&f.operator, &f.pool, 4), Err(UNAUTHORIZED));
    assert_eq!(f.pool.status(), 6);
}

#[test]
fn operator_cannot_call_owner_functions() {
    let f = Fixture::adopted();

    let results = f.try_owner_fns(&f.operator);
    for (name, result) in OWNER_FNS.iter().zip(results) {
        assert_eq!(result, Err(UNAUTHORIZED), "{name}");
    }
    assert_eq!(f.manager.get_operator(), f.operator);
    assert_eq!(f.pool.admin(), f.manager.address);
    assert_eq!(f.pool.config(), None);
    assert_eq!(f.pool.emissions(), None);
}

#[test]
fn other_signer_cannot_call_owner_functions() {
    let f = Fixture::adopted();
    let other = Address::generate(&f.e);

    let results = f.try_owner_fns(&other);
    for (name, result) in OWNER_FNS.iter().zip(results) {
        assert_eq!(result, Err(UNAUTHORIZED), "{name}");
    }
}

#[test]
fn owner_passes_admin_calls_through() {
    let f = Fixture::adopted();
    let p = &f.pool.address;
    let asset = Address::generate(&f.e);
    let config = reserve_config();

    f.sign(
        &f.owner,
        "update_pool",
        (p, 2_000_000u32, 6u32, 10_000_000i128).into_val(&f.e),
    );
    f.manager.update_pool(p, &2_000_000, &6, &10_000_000);
    assert_eq!(f.pool.config(), Some((2_000_000, 6, 10_000_000)));

    f.sign(
        &f.owner,
        "queue_set_reserve",
        (p, &asset, config.clone()).into_val(&f.e),
    );
    f.manager.queue_set_reserve(p, &asset, &config);
    assert_eq!(f.pool.queued(&asset), Some(config));

    f.sign(&f.owner, "cancel_set_reserve", (p, &asset).into_val(&f.e));
    f.manager.cancel_set_reserve(p, &asset);
    assert_eq!(f.pool.queued(&asset), None);

    let emissions = vec![
        &f.e,
        ReserveEmissionMetadata {
            res_index: 1,
            res_type: 0,
            share: 5_000_000,
        },
    ];
    f.sign(
        &f.owner,
        "set_emissions_config",
        (p, emissions.clone()).into_val(&f.e),
    );
    f.manager.set_emissions_config(p, &emissions);
    assert_eq!(f.pool.emissions(), Some(emissions));
}

#[test]
fn owner_replaces_operator() {
    let f = Fixture::adopted();
    let new_operator = Address::generate(&f.e);

    f.sign(&f.owner, "set_operator", (&new_operator,).into_val(&f.e));
    f.manager.set_operator(&new_operator);
    let events = f.e.events().all();
    assert_eq!(events.events().len(), 1);
    assert_eq!(f.manager.get_operator(), new_operator);

    assert_eq!(
        f.try_set_status(&f.operator, &f.pool, 0),
        Err(NOT_OWNER_OR_OPERATOR)
    );
    assert_eq!(f.try_set_status(&new_operator, &f.pool, 0), Ok(()));
    assert_eq!(f.pool.status(), 0);
}

#[test]
fn owner_hands_pool_to_another_admin() {
    let f = Fixture::adopted();
    let next = Address::generate(&f.e);

    f.sign(
        &f.owner,
        "propose_admin",
        (&f.pool.address, &next).into_val(&f.e),
    );
    f.manager.propose_admin(&f.pool.address, &next);
    f.e.mock_all_auths();
    f.pool.accept_admin();
    assert_eq!(f.pool.admin(), next);

    f.e.set_auths(&[]);
    assert_eq!(f.try_set_status(&f.operator, &f.pool, 0), Err(UNAUTHORIZED));
}

#[test]
fn manages_several_pools() {
    let f = Fixture::adopted();
    let other_pool = Fixture::new_pool(&f.e);
    f.propose(&other_pool);
    f.try_accept(&f.owner, &other_pool).unwrap();

    f.try_set_status(&f.operator, &f.pool, 4).unwrap();
    f.try_set_status(&f.operator, &other_pool, 0).unwrap();

    assert_eq!(f.pool.status(), 4);
    assert_eq!(other_pool.status(), 0);
}

#[test]
fn exposes_no_ownership_or_upgrade_entrypoints() {
    let f = Fixture::new();

    f.e.as_contract(&f.manager.address, || {
        assert!(PoolStatusManager
            .call("get_owner", f.e.clone(), &[])
            .is_some());
        for function in [
            "transfer_ownership",
            "accept_ownership",
            "renounce_ownership",
            "set_reserve",
            "upgrade",
            "__check_auth",
        ] {
            assert!(
                PoolStatusManager.call(function, f.e.clone(), &[]).is_none(),
                "{function}"
            );
        }
    });
}
