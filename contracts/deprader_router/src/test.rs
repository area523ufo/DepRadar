use super::{
    BountyStatus, ClaimStatus, DepRaderRouter, DepRaderRouterClient, Error,
    MAX_BOUNTY_DURATION_SECS, MAX_CLAIMS_PER_BOUNTY, REVIEW_WINDOW_SECS,
};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::{StellarAssetClient, TokenClient},
    Address, Env, String, Symbol, TryFromVal,
};

const START: u64 = 1_000_000;
const DEADLINE: u64 = START + 30 * 24 * 60 * 60;
const AMOUNT: i128 = 400;

struct Setup<'a> {
    env: Env,
    client: DepRaderRouterClient<'a>,
    token: TokenClient<'a>,
    admin: Address,
    creator: Address,
}

/// Fresh env at ledger time `START` with an initialized router and a token
/// in which `creator` holds 1_000 units. All auths are mocked; tests that
/// care about *who* authorized a call check `env.auths()`.
fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let admin = Address::generate(&env);
    let creator = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(token_admin);
    let token = TokenClient::new(&env, &sac.address());
    StellarAssetClient::new(&env, &sac.address()).mint(&creator, &1_000);

    let contract_id = env.register(DepRaderRouter, ());
    let client = DepRaderRouterClient::new(&env, &contract_id);
    client.initialize(&admin);

    Setup {
        env,
        client,
        token,
        admin,
        creator,
    }
}

impl Setup<'_> {
    fn s(&self, v: &str) -> String {
        String::from_str(&self.env, v)
    }

    fn create(&self) -> u64 {
        self.client.create_bounty(
            &self.creator,
            &self.s("org/repo#123"),
            &AMOUNT,
            &self.token.address,
            &DEADLINE,
        )
    }

    fn claim(&self, bounty_id: u64) -> (Address, u32) {
        let claimant = Address::generate(&self.env);
        let id = self
            .client
            .submit_claim(&bounty_id, &claimant, &self.s("org/repo#123-pr-456"));
        (claimant, id)
    }

    fn last_event_topic(&self) -> Symbol {
        let (_, topics, _) = self.env.events().all().last().expect("an event");
        Symbol::try_from_val(&self.env, &topics.get(0).unwrap()).unwrap()
    }

    /// The single address whose authorization the last invocation required.
    fn last_auth(&self) -> Address {
        let auths = self.env.auths();
        assert_eq!(auths.len(), 1, "expected exactly one authorizer");
        auths[0].0.clone()
    }
}

// --------------------------------------------------------------- initialize

#[test]
fn test_initialize_twice_fails() {
    let t = setup();
    let other = Address::generate(&t.env);
    assert_eq!(
        t.client.try_initialize(&other),
        Err(Ok(Error::AlreadyInitialized))
    );
    assert_eq!(t.client.get_admin(), Some(t.admin.clone()));
}

#[test]
fn test_create_requires_initialized_contract() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);
    let creator = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let client = DepRaderRouterClient::new(&env, &env.register(DepRaderRouter, ()));

    let result = client.try_create_bounty(
        &creator,
        &String::from_str(&env, "org/repo#1"),
        &AMOUNT,
        &token,
        &DEADLINE,
    );
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

// ------------------------------------------------------------ create_bounty

#[test]
fn test_create_bounty_locks_funds_and_assigns_ids() {
    let t = setup();

    let first = t.create();
    assert_eq!(t.last_event_topic(), symbol_short!("created"));
    let second = t.create();

    assert_eq!((first, second), (1, 2));
    assert_eq!(t.client.bounty_count(), 2);
    assert_eq!(t.token.balance(&t.creator), 1_000 - 2 * AMOUNT);
    assert_eq!(t.token.balance(&t.client.address), 2 * AMOUNT);

    let bounty = t.client.get_bounty(&first).expect("bounty should exist");
    assert_eq!(bounty.creator, t.creator);
    assert_eq!(bounty.issue_ref, t.s("org/repo#123"));
    assert_eq!(bounty.amount, AMOUNT);
    assert_eq!(bounty.token, t.token.address);
    assert_eq!(bounty.status, BountyStatus::Open);
    assert_eq!(bounty.created_at, START);
    assert_eq!(bounty.deadline, DEADLINE);
    assert_eq!(bounty.claim_count, 0);
}

#[test]
fn test_create_bounty_requires_creator_auth() {
    let t = setup();
    t.create();
    assert_eq!(t.last_auth(), t.creator);
}

#[test]
fn test_create_bounty_rejects_bad_input() {
    let t = setup();
    let create = |r: &str, amount: i128, deadline: u64| {
        t.client
            .try_create_bounty(&t.creator, &t.s(r), &amount, &t.token.address, &deadline)
    };

    assert_eq!(create("x", 0, DEADLINE), Err(Ok(Error::InvalidAmount)));
    assert_eq!(create("x", -5, DEADLINE), Err(Ok(Error::InvalidAmount)));
    assert_eq!(
        create("", AMOUNT, DEADLINE),
        Err(Ok(Error::InvalidReference))
    );
    let long = "a".repeat(257);
    assert_eq!(
        create(&long, AMOUNT, DEADLINE),
        Err(Ok(Error::InvalidReference))
    );
    assert_eq!(create("x", AMOUNT, START), Err(Ok(Error::InvalidDeadline)));
    assert_eq!(
        create("x", AMOUNT, START + MAX_BOUNTY_DURATION_SECS + 1),
        Err(Ok(Error::InvalidDeadline))
    );

    // Nothing was locked and no ID was consumed.
    assert_eq!(t.token.balance(&t.client.address), 0);
    assert_eq!(t.client.bounty_count(), 0);
}

// ------------------------------------------------------------- submit_claim

#[test]
fn test_submit_claim_records_without_moving_funds() {
    let t = setup();
    let bounty_id = t.create();
    let (claimant, claim_id) = t.claim(bounty_id);

    assert_eq!(claim_id, 0);
    assert_eq!(t.last_auth(), claimant);
    assert_eq!(t.last_event_topic(), symbol_short!("claimed"));
    assert_eq!(t.token.balance(&t.client.address), AMOUNT);

    let claim = t.client.get_claim(&bounty_id, &claim_id).unwrap();
    assert_eq!(claim.claimant, claimant);
    assert_eq!(claim.proof_ref, t.s("org/repo#123-pr-456"));
    assert_eq!(claim.submitted_at, START);
    assert_eq!(claim.status, ClaimStatus::Pending);
    assert_eq!(t.client.get_claim_id(&bounty_id, &claimant), Some(claim_id));

    let bounty = t.client.get_bounty(&bounty_id).unwrap();
    assert_eq!((bounty.claim_count, bounty.pending_claims), (1, 1));
}

#[test]
fn test_multiple_claimants_can_claim_one_bounty() {
    let t = setup();
    let bounty_id = t.create();
    let (_, a) = t.claim(bounty_id);
    let (_, b) = t.claim(bounty_id);
    let (_, c) = t.claim(bounty_id);

    assert_eq!((a, b, c), (0, 1, 2));
    assert_eq!(t.client.get_bounty(&bounty_id).unwrap().pending_claims, 3);
}

#[test]
fn test_submit_claim_rejects_duplicates_and_bad_input() {
    let t = setup();
    let bounty_id = t.create();
    let (claimant, _) = t.claim(bounty_id);

    assert_eq!(
        t.client
            .try_submit_claim(&bounty_id, &claimant, &t.s("again")),
        Err(Ok(Error::DuplicateClaim))
    );
    let someone = Address::generate(&t.env);
    assert_eq!(
        t.client.try_submit_claim(&bounty_id, &someone, &t.s("")),
        Err(Ok(Error::InvalidReference))
    );
    assert_eq!(
        t.client.try_submit_claim(&999, &someone, &t.s("pr")),
        Err(Ok(Error::BountyNotFound))
    );
    assert!(t.client.get_claim(&999, &0).is_none());
}

#[test]
fn test_submit_claim_after_deadline_fails() {
    let t = setup();
    let bounty_id = t.create();
    t.env.ledger().set_timestamp(DEADLINE);

    let claimant = Address::generate(&t.env);
    assert_eq!(
        t.client
            .try_submit_claim(&bounty_id, &claimant, &t.s("late")),
        Err(Ok(Error::ClaimWindowClosed))
    );
}

#[test]
fn test_submit_claim_is_capped_per_bounty() {
    let t = setup();
    let bounty_id = t.create();
    for _ in 0..MAX_CLAIMS_PER_BOUNTY {
        t.claim(bounty_id);
    }
    let one_more = Address::generate(&t.env);
    assert_eq!(
        t.client.try_submit_claim(&bounty_id, &one_more, &t.s("pr")),
        Err(Ok(Error::TooManyClaims))
    );
}

// ------------------------------------------------------ reject / release

#[test]
fn test_release_pays_chosen_claim() {
    let t = setup();
    let bounty_id = t.create();
    let (_, first) = t.claim(bounty_id);
    let (winner, second) = t.claim(bounty_id);

    t.client.release(&bounty_id, &second);
    assert_eq!(t.last_auth(), t.admin);
    assert_eq!(t.last_event_topic(), symbol_short!("released"));

    assert_eq!(t.token.balance(&winner), AMOUNT);
    assert_eq!(t.token.balance(&t.client.address), 0);
    let bounty = t.client.get_bounty(&bounty_id).unwrap();
    assert_eq!(bounty.status, BountyStatus::Released);
    assert_eq!(
        t.client.get_claim(&bounty_id, &second).unwrap().status,
        ClaimStatus::Accepted
    );
    // The other claim was never decided on, and now can't be.
    assert_eq!(
        t.client.get_claim(&bounty_id, &first).unwrap().status,
        ClaimStatus::Pending
    );
    assert_eq!(
        t.client.try_release(&bounty_id, &first),
        Err(Ok(Error::BountyNotOpen))
    );
}

#[test]
fn test_squatted_claim_can_be_rejected_and_real_claim_paid() {
    // The scenario the old single-claim design couldn't handle: a junk
    // claim lands first, the real finder claims second.
    let t = setup();
    let bounty_id = t.create();
    let (squatter, junk) = t.claim(bounty_id);
    let (finder, real) = t.claim(bounty_id);

    t.client.reject_claim(&bounty_id, &junk);
    assert_eq!(t.last_auth(), t.admin);
    assert_eq!(t.last_event_topic(), symbol_short!("rejected"));
    assert_eq!(
        t.client.get_claim(&bounty_id, &junk).unwrap().status,
        ClaimStatus::Rejected
    );
    assert_eq!(t.client.get_bounty(&bounty_id).unwrap().pending_claims, 1);

    // A rejected claim can't be paid or re-submitted.
    assert_eq!(
        t.client.try_release(&bounty_id, &junk),
        Err(Ok(Error::ClaimNotPending))
    );
    assert_eq!(
        t.client
            .try_submit_claim(&bounty_id, &squatter, &t.s("retry")),
        Err(Ok(Error::DuplicateClaim))
    );

    t.client.release(&bounty_id, &real);
    assert_eq!(t.token.balance(&finder), AMOUNT);
    assert_eq!(t.token.balance(&squatter), 0);
}

#[test]
fn test_release_works_during_review_window() {
    let t = setup();
    let bounty_id = t.create();
    let (claimant, claim_id) = t.claim(bounty_id);

    t.env.ledger().set_timestamp(DEADLINE + 60);
    t.client.release(&bounty_id, &claim_id);
    assert_eq!(t.token.balance(&claimant), AMOUNT);
}

#[test]
fn test_release_and_reject_errors() {
    let t = setup();
    let bounty_id = t.create();

    assert_eq!(
        t.client.try_release(&bounty_id, &0),
        Err(Ok(Error::ClaimNotFound))
    );
    assert_eq!(
        t.client.try_reject_claim(&bounty_id, &0),
        Err(Ok(Error::ClaimNotFound))
    );
    assert_eq!(
        t.client.try_release(&999, &0),
        Err(Ok(Error::BountyNotFound))
    );

    let (_, claim_id) = t.claim(bounty_id);
    t.client.release(&bounty_id, &claim_id);
    assert_eq!(
        t.client.try_release(&bounty_id, &claim_id),
        Err(Ok(Error::BountyNotOpen))
    );
}

#[test]
fn test_release_without_admin_auth_fails() {
    let t = setup();
    let bounty_id = t.create();
    let (claimant, claim_id) = t.claim(bounty_id);

    // Drop the blanket auth mock: nobody has signed anything.
    t.env.set_auths(&[]);
    assert!(t.client.try_release(&bounty_id, &claim_id).is_err());
    assert!(t.client.try_reject_claim(&bounty_id, &claim_id).is_err());

    assert_eq!(t.token.balance(&claimant), 0);
    assert_eq!(t.token.balance(&t.client.address), AMOUNT);
    assert_eq!(
        t.client.get_bounty(&bounty_id).unwrap().status,
        BountyStatus::Open
    );
}

// ------------------------------------------------------------ cancel_bounty

#[test]
fn test_admin_can_cancel_any_time() {
    let t = setup();
    let bounty_id = t.create();
    t.claim(bounty_id);

    t.client.cancel_bounty(&t.admin, &bounty_id);
    assert_eq!(t.last_event_topic(), symbol_short!("cancelled"));
    assert_eq!(t.token.balance(&t.creator), 1_000);
    assert_eq!(
        t.client.get_bounty(&bounty_id).unwrap().status,
        BountyStatus::Cancelled
    );

    // A cancelled bounty takes no more claims and can't be paid.
    let late = Address::generate(&t.env);
    assert_eq!(
        t.client.try_submit_claim(&bounty_id, &late, &t.s("pr")),
        Err(Ok(Error::BountyNotOpen))
    );
    assert_eq!(
        t.client.try_release(&bounty_id, &0),
        Err(Ok(Error::BountyNotOpen))
    );
    assert_eq!(
        t.client.try_cancel_bounty(&t.admin, &bounty_id),
        Err(Ok(Error::BountyNotOpen))
    );
}

#[test]
fn test_creator_cannot_cancel_before_deadline() {
    let t = setup();
    let bounty_id = t.create();
    assert_eq!(
        t.client.try_cancel_bounty(&t.creator, &bounty_id),
        Err(Ok(Error::CancelTooEarly))
    );
}

#[test]
fn test_creator_can_cancel_after_deadline_with_no_pending_claims() {
    let t = setup();
    let bounty_id = t.create();
    let (_, junk) = t.claim(bounty_id);
    t.client.reject_claim(&bounty_id, &junk);

    t.env.ledger().set_timestamp(DEADLINE);
    t.client.cancel_bounty(&t.creator, &bounty_id);
    assert_eq!(t.last_auth(), t.creator);
    assert_eq!(t.token.balance(&t.creator), 1_000);
}

#[test]
fn test_pending_claim_holds_funds_until_review_window_ends() {
    let t = setup();
    let bounty_id = t.create();
    t.claim(bounty_id);

    t.env.ledger().set_timestamp(DEADLINE);
    assert_eq!(
        t.client.try_cancel_bounty(&t.creator, &bounty_id),
        Err(Ok(Error::CancelTooEarly))
    );

    t.env
        .ledger()
        .set_timestamp(DEADLINE + REVIEW_WINDOW_SECS - 1);
    assert_eq!(
        t.client.try_cancel_bounty(&t.creator, &bounty_id),
        Err(Ok(Error::CancelTooEarly))
    );

    // Admin never reviewed: the creator gets the funds back.
    t.env.ledger().set_timestamp(DEADLINE + REVIEW_WINDOW_SECS);
    t.client.cancel_bounty(&t.creator, &bounty_id);
    assert_eq!(t.token.balance(&t.creator), 1_000);
}

#[test]
fn test_stranger_cannot_cancel() {
    let t = setup();
    let bounty_id = t.create();
    t.env.ledger().set_timestamp(DEADLINE + REVIEW_WINDOW_SECS);

    let stranger = Address::generate(&t.env);
    assert_eq!(
        t.client.try_cancel_bounty(&stranger, &bounty_id),
        Err(Ok(Error::NotAuthorized))
    );
    assert_eq!(t.token.balance(&t.client.address), AMOUNT);
}

// ----------------------------------------------------------- admin rotation

#[test]
fn test_admin_rotation_is_two_step() {
    let t = setup();
    let new_admin = Address::generate(&t.env);

    assert_eq!(t.client.try_accept_admin(), Err(Ok(Error::NoPendingAdmin)));

    t.client.propose_admin(&new_admin);
    assert_eq!(t.last_auth(), t.admin);
    // Proposing alone changes nothing.
    assert_eq!(t.client.get_admin(), Some(t.admin.clone()));
    assert_eq!(t.client.get_pending_admin(), Some(new_admin.clone()));

    t.client.accept_admin();
    assert_eq!(t.last_auth(), new_admin);
    assert_eq!(t.client.get_admin(), Some(new_admin.clone()));
    assert_eq!(t.client.get_pending_admin(), None);

    // Admin-gated calls now need the new admin's signature.
    let bounty_id = t.create();
    let (_, claim_id) = t.claim(bounty_id);
    t.client.release(&bounty_id, &claim_id);
    assert_eq!(t.last_auth(), new_admin);
}

// --------------------------------------------------------------- min_amount

#[test]
fn test_min_amount_per_token_enforced_at_below_and_above() {
    let t = setup();

    // Default minimum is 0 for any token.
    assert_eq!(t.client.get_min_amount(&t.token.address), 0);

    // Negative min_amount is rejected.
    assert_eq!(
        t.client.try_set_min_amount(&t.token.address, &-1),
        Err(Ok(Error::InvalidAmount))
    );

    // Admin sets min_amount = 100 for `t.token`.
    t.client.set_min_amount(&t.token.address, &100);
    assert_eq!(t.last_auth(), t.admin);
    assert_eq!(t.last_event_topic(), symbol_short!("min_amt"));
    assert_eq!(t.client.get_min_amount(&t.token.address), 100);

    // Below minimum (99) is rejected with AmountTooSmall; no funds move and no ID is consumed.
    assert_eq!(
        t.client.try_create_bounty(
            &t.creator,
            &t.s("org/repo#below"),
            &99,
            &t.token.address,
            &DEADLINE,
        ),
        Err(Ok(Error::AmountTooSmall))
    );
    assert_eq!(t.client.bounty_count(), 0);
    assert_eq!(t.token.balance(&t.client.address), 0);

    // Exactly at minimum (100) succeeds.
    let at_min_id = t.client.create_bounty(
        &t.creator,
        &t.s("org/repo#at-min"),
        &100,
        &t.token.address,
        &DEADLINE,
    );
    assert_eq!(at_min_id, 1);

    // Above minimum (150) succeeds.
    let above_min_id = t.client.create_bounty(
        &t.creator,
        &t.s("org/repo#above-min"),
        &150,
        &t.token.address,
        &DEADLINE,
    );
    assert_eq!(above_min_id, 2);
    assert_eq!(t.token.balance(&t.client.address), 250);

    // Another token still defaults to 0 until configured.
    let other_sac = t
        .env
        .register_stellar_asset_contract_v2(Address::generate(&t.env));
    StellarAssetClient::new(&t.env, &other_sac.address()).mint(&t.creator, &50);
    assert_eq!(t.client.get_min_amount(&other_sac.address()), 0);
    let other_id = t.client.create_bounty(
        &t.creator,
        &t.s("org/repo#other-token"),
        &1,
        &other_sac.address(),
        &DEADLINE,
    );
    assert_eq!(other_id, 3);
}
