#![no_std]

//! # deprader_router
//!
//! On-chain bounty router for DepRadar. A creator locks funds against a
//! security finding, researchers submit claims with a reference to their
//! proof, and the admin either releases the bounty to one specific claim or
//! rejects bad claims. If nobody is paid, the creator can reclaim the funds
//! once the bounty's deadline (plus a review window) has passed.
//!
//! ```text
//! create_bounty ──► Open ──submit_claim──► claim: Pending
//!                    │                        ├─ reject_claim → Rejected (bounty stays Open)
//!                    │                        └─ release      → Accepted, bounty Released
//!                    └─ cancel_bounty ──► Cancelled (funds back to creator)
//! ```

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env,
    IntoVal, String, TryFromVal, Val,
};

/// Ledgers per day at Stellar's ~5s close time.
const DAY_IN_LEDGERS: u32 = 17_280;
const INSTANCE_TTL_THRESHOLD: u32 = 29 * DAY_IN_LEDGERS;
const INSTANCE_TTL_EXTEND_TO: u32 = 30 * DAY_IN_LEDGERS;
const PERSISTENT_TTL_THRESHOLD: u32 = 60 * DAY_IN_LEDGERS;
const PERSISTENT_TTL_EXTEND_TO: u32 = 90 * DAY_IN_LEDGERS;

/// Seconds after the deadline during which the admin can still review
/// pending claims before the creator may reclaim the funds.
pub const REVIEW_WINDOW_SECS: u64 = 7 * 24 * 60 * 60;
/// Longest a bounty may stay open for claims.
pub const MAX_BOUNTY_DURATION_SECS: u64 = 365 * 24 * 60 * 60;
/// Upper bound on claims per bounty, so one bounty can't be spammed into
/// unbounded storage.
pub const MAX_CLAIMS_PER_BOUNTY: u32 = 50;
/// Upper bound (in bytes) for `issue_ref` / `proof_ref`.
pub const MAX_REF_LEN: u32 = 256;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BountyStatus {
    Open,
    Released,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaimStatus {
    Pending,
    Rejected,
    Accepted,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bounty {
    pub creator: Address,
    pub issue_ref: String,
    pub amount: i128,
    pub token: Address,
    pub status: BountyStatus,
    /// Ledger timestamp the bounty was created at.
    pub created_at: u64,
    /// Ledger timestamp after which no new claims are accepted.
    pub deadline: u64,
    /// Total claims ever submitted; claim IDs are `0..claim_count`.
    pub claim_count: u32,
    /// Claims still awaiting an admin decision.
    pub pending_claims: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Claim {
    pub claimant: Address,
    pub proof_ref: String,
    pub submitted_at: u64,
    pub status: ClaimStatus,
}

#[contracttype]
enum DataKey {
    Admin,
    PendingAdmin,
    NextBountyId,
    Bounty(u64),
    Claim(u64, u32),
    ClaimBy(u64, Address),
    MinAmount(Address),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum Error {
    InvalidAmount = 1,
    BountyNotFound = 3,
    AlreadyInitialized = 5,
    NotInitialized = 6,
    NotAuthorized = 7,
    InvalidDeadline = 10,
    InvalidReference = 11,
    BountyNotOpen = 12,
    ClaimWindowClosed = 13,
    DuplicateClaim = 14,
    TooManyClaims = 15,
    ClaimNotFound = 16,
    ClaimNotPending = 17,
    CancelTooEarly = 18,
    NoPendingAdmin = 19,
    AmountTooSmall = 20,
}

#[contract]
pub struct DepRaderRouter;

#[contractimpl]
impl DepRaderRouter {
    // ---------------------------------------------------------------- admin

    /// Sets the contract admin. Must be called once before any bounty can be
    /// created, so funds are never locked in a contract nobody can resolve.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }

        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        bump_instance(&env);

        env.events().publish((symbol_short!("admin_set"),), admin);
        Ok(())
    }

    /// First step of an admin rotation: the current admin nominates a
    /// successor. Nothing changes until the successor accepts.
    pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        let admin = require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::PendingAdmin, &new_admin);

        env.events()
            .publish((symbol_short!("admin_prp"), admin), new_admin);
        Ok(())
    }

    /// Second step of an admin rotation: the nominee accepts and becomes the
    /// admin. Requiring the nominee's signature means a mistyped address
    /// can't lock everyone out of the contract.
    pub fn accept_admin(env: Env) -> Result<(), Error> {
        let pending: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(Error::NoPendingAdmin)?;
        pending.require_auth();

        env.storage().instance().set(&DataKey::Admin, &pending);
        env.storage().instance().remove(&DataKey::PendingAdmin);
        bump_instance(&env);

        env.events().publish((symbol_short!("admin_set"),), pending);
        Ok(())
    }

    pub fn get_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::Admin)
    }

    pub fn get_pending_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::PendingAdmin)
    }

    /// Admin-only: sets the minimum bounty amount required for `token`
    /// (default is `0` when unset). Rejects negative values with
    /// [`Error::InvalidAmount`].
    pub fn set_min_amount(env: Env, token: Address, min_amount: i128) -> Result<(), Error> {
        let admin = require_admin(&env)?;
        if min_amount < 0 {
            return Err(Error::InvalidAmount);
        }

        set_persistent(&env, &DataKey::MinAmount(token.clone()), &min_amount);
        env.events()
            .publish((symbol_short!("min_amt"), token), (min_amount, admin));
        Ok(())
    }

    /// Returns the configured minimum bounty amount for `token`, or `0` if
    /// none has been set.
    pub fn get_min_amount(env: Env, token: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::MinAmount(token))
            .unwrap_or(0)
    }

    // ------------------------------------------------------------- bounties

    /// Locks `amount` of `token` from `creator` against a new bounty and
    /// returns its contract-assigned ID. Claims are accepted until
    /// `deadline` (a ledger timestamp, in seconds).
    pub fn create_bounty(
        env: Env,
        creator: Address,
        issue_ref: String,
        amount: i128,
        token: Address,
        deadline: u64,
    ) -> Result<u64, Error> {
        creator.require_auth();

        stored_admin(&env)?;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let min_amount: i128 =
            get_persistent(&env, &DataKey::MinAmount(token.clone())).unwrap_or(0);
        if amount < min_amount {
            return Err(Error::AmountTooSmall);
        }
        check_ref(&issue_ref)?;
        let now = env.ledger().timestamp();
        if deadline <= now || deadline - now > MAX_BOUNTY_DURATION_SECS {
            return Err(Error::InvalidDeadline);
        }

        let bounty_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextBountyId)
            .unwrap_or(1);
        env.storage()
            .instance()
            .set(&DataKey::NextBountyId, &(bounty_id + 1));
        bump_instance(&env);

        let bounty = Bounty {
            creator: creator.clone(),
            issue_ref,
            amount,
            token: token.clone(),
            status: BountyStatus::Open,
            created_at: now,
            deadline,
            claim_count: 0,
            pending_claims: 0,
        };
        set_persistent(&env, &DataKey::Bounty(bounty_id), &bounty);

        token::Client::new(&env, &token).transfer(
            &creator,
            &env.current_contract_address(),
            &amount,
        );

        env.events().publish(
            (symbol_short!("created"), bounty_id),
            (creator, token, amount, deadline),
        );
        Ok(bounty_id)
    }

    /// Reads back the stored state for a bounty, if one exists.
    pub fn get_bounty(env: Env, bounty_id: u64) -> Option<Bounty> {
        env.storage().persistent().get(&DataKey::Bounty(bounty_id))
    }

    /// Number of bounties created so far. Bounty IDs are `1..=bounty_count`.
    pub fn bounty_count(env: Env) -> u64 {
        let next: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextBountyId)
            .unwrap_or(1);
        next - 1
    }

    /// Refunds an open bounty to its creator.
    ///
    /// - The admin may cancel at any time (e.g. the finding was invalid).
    /// - The creator may cancel once the deadline has passed with no pending
    ///   claims, or once the review window after the deadline has also
    ///   passed. Funds can never be locked forever, but a creator also can't
    ///   pull them out from under a claimant who is still awaiting review.
    pub fn cancel_bounty(env: Env, caller: Address, bounty_id: u64) -> Result<(), Error> {
        caller.require_auth();

        let key = DataKey::Bounty(bounty_id);
        let mut bounty: Bounty = get_persistent(&env, &key).ok_or(Error::BountyNotFound)?;
        if bounty.status != BountyStatus::Open {
            return Err(Error::BountyNotOpen);
        }

        if stored_admin(&env)? != caller {
            if caller != bounty.creator {
                return Err(Error::NotAuthorized);
            }
            let now = env.ledger().timestamp();
            let deadline_passed = now >= bounty.deadline;
            let review_over = now >= bounty.deadline.saturating_add(REVIEW_WINDOW_SECS);
            if !(review_over || (deadline_passed && bounty.pending_claims == 0)) {
                return Err(Error::CancelTooEarly);
            }
        }

        // State first, transfer last.
        bounty.status = BountyStatus::Cancelled;
        set_persistent(&env, &key, &bounty);

        token::Client::new(&env, &bounty.token).transfer(
            &env.current_contract_address(),
            &bounty.creator,
            &bounty.amount,
        );

        env.events().publish(
            (symbol_short!("cancelled"), bounty_id),
            (caller, bounty.amount),
        );
        Ok(())
    }

    // --------------------------------------------------------------- claims

    /// Records a claim against an open bounty and returns its claim ID.
    /// Moves no funds. Any number of researchers may claim the same bounty
    /// (up to [`MAX_CLAIMS_PER_BOUNTY`], one claim per address), so a junk
    /// claim submitted first can't block a real one.
    pub fn submit_claim(
        env: Env,
        bounty_id: u64,
        claimant: Address,
        proof_ref: String,
    ) -> Result<u32, Error> {
        claimant.require_auth();
        check_ref(&proof_ref)?;

        let key = DataKey::Bounty(bounty_id);
        let mut bounty: Bounty = get_persistent(&env, &key).ok_or(Error::BountyNotFound)?;
        if bounty.status != BountyStatus::Open {
            return Err(Error::BountyNotOpen);
        }
        let now = env.ledger().timestamp();
        if now >= bounty.deadline {
            return Err(Error::ClaimWindowClosed);
        }
        let by_key = DataKey::ClaimBy(bounty_id, claimant.clone());
        if env.storage().persistent().has(&by_key) {
            return Err(Error::DuplicateClaim);
        }
        if bounty.claim_count >= MAX_CLAIMS_PER_BOUNTY {
            return Err(Error::TooManyClaims);
        }

        let claim_id = bounty.claim_count;
        let claim = Claim {
            claimant: claimant.clone(),
            proof_ref,
            submitted_at: now,
            status: ClaimStatus::Pending,
        };
        set_persistent(&env, &DataKey::Claim(bounty_id, claim_id), &claim);
        set_persistent(&env, &by_key, &claim_id);

        bounty.claim_count += 1;
        bounty.pending_claims += 1;
        set_persistent(&env, &key, &bounty);

        env.events()
            .publish((symbol_short!("claimed"), bounty_id), (claim_id, claimant));
        Ok(claim_id)
    }

    /// Reads back one claim on a bounty, if it exists.
    pub fn get_claim(env: Env, bounty_id: u64, claim_id: u32) -> Option<Claim> {
        env.storage()
            .persistent()
            .get(&DataKey::Claim(bounty_id, claim_id))
    }

    /// Looks up the claim ID `claimant` holds on a bounty, if any.
    pub fn get_claim_id(env: Env, bounty_id: u64, claimant: Address) -> Option<u32> {
        env.storage()
            .persistent()
            .get(&DataKey::ClaimBy(bounty_id, claimant))
    }

    /// Admin-only: marks a pending claim as rejected. The bounty stays open
    /// for other claims.
    pub fn reject_claim(env: Env, bounty_id: u64, claim_id: u32) -> Result<(), Error> {
        let admin = require_admin(&env)?;
        let (mut bounty, mut claim) = load_open_pending(&env, bounty_id, claim_id)?;

        claim.status = ClaimStatus::Rejected;
        set_persistent(&env, &DataKey::Claim(bounty_id, claim_id), &claim);
        bounty.pending_claims -= 1;
        set_persistent(&env, &DataKey::Bounty(bounty_id), &bounty);

        env.events()
            .publish((symbol_short!("rejected"), bounty_id), (claim_id, admin));
        Ok(())
    }

    /// Admin-only: pays the locked funds to the chosen claim's claimant and
    /// closes the bounty. Works after the deadline too, so claims submitted
    /// in time can still be paid during the review window.
    pub fn release(env: Env, bounty_id: u64, claim_id: u32) -> Result<(), Error> {
        let admin = require_admin(&env)?;
        let (mut bounty, mut claim) = load_open_pending(&env, bounty_id, claim_id)?;

        // State first, transfer last.
        claim.status = ClaimStatus::Accepted;
        set_persistent(&env, &DataKey::Claim(bounty_id, claim_id), &claim);
        bounty.status = BountyStatus::Released;
        bounty.pending_claims -= 1;
        set_persistent(&env, &DataKey::Bounty(bounty_id), &bounty);

        token::Client::new(&env, &bounty.token).transfer(
            &env.current_contract_address(),
            &claim.claimant,
            &bounty.amount,
        );

        env.events().publish(
            (symbol_short!("released"), bounty_id),
            (claim_id, claim.claimant, bounty.amount, admin),
        );
        Ok(())
    }
}

// ------------------------------------------------------------------ helpers

fn stored_admin(env: &Env) -> Result<Address, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)
}

fn require_admin(env: &Env) -> Result<Address, Error> {
    let admin = stored_admin(env)?;
    admin.require_auth();
    bump_instance(env);
    Ok(admin)
}

/// Loads a bounty that must be `Open` and one of its claims that must be
/// `Pending` — the precondition shared by `reject_claim` and `release`.
fn load_open_pending(env: &Env, bounty_id: u64, claim_id: u32) -> Result<(Bounty, Claim), Error> {
    let bounty: Bounty =
        get_persistent(env, &DataKey::Bounty(bounty_id)).ok_or(Error::BountyNotFound)?;
    if bounty.status != BountyStatus::Open {
        return Err(Error::BountyNotOpen);
    }
    let claim: Claim =
        get_persistent(env, &DataKey::Claim(bounty_id, claim_id)).ok_or(Error::ClaimNotFound)?;
    if claim.status != ClaimStatus::Pending {
        return Err(Error::ClaimNotPending);
    }
    Ok((bounty, claim))
}

fn check_ref(r: &String) -> Result<(), Error> {
    if r.is_empty() || r.len() > MAX_REF_LEN {
        return Err(Error::InvalidReference);
    }
    Ok(())
}

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
}

/// Writes a persistent entry and extends its TTL so live bounties and
/// claims don't get archived mid-lifecycle.
fn set_persistent<V: IntoVal<Env, Val>>(env: &Env, key: &DataKey, value: &V) {
    env.storage().persistent().set(key, value);
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_EXTEND_TO);
}

/// Reads a persistent entry inside a state-changing call, extending its TTL
/// if present. Public getters read without extending.
fn get_persistent<V: TryFromVal<Env, Val>>(env: &Env, key: &DataKey) -> Option<V> {
    let value = env.storage().persistent().get(key);
    if value.is_some() {
        env.storage().persistent().extend_ttl(
            key,
            PERSISTENT_TTL_THRESHOLD,
            PERSISTENT_TTL_EXTEND_TO,
        );
    }
    value
}

#[cfg(test)]
mod test;
