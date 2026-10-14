# Progress Log

This file is the running record of concrete work landed in this repo: what
was built, who built it, and which issue (if any) it closes. It exists so
that "what's actually shipped" is answerable by reading one file, rather
than reconstructing it from git history or a scattered issue tracker.

Every issue's Definition of Done requires an entry here before the issue
is closed — see [CONTRIBUTING.md](CONTRIBUTING.md).

## Entries

### deprader_router contract — create_bounty / submit_claim / release

- **What:** Initial Soroban smart contract at `contracts/deprader_router/`.
  Implements `initialize(admin)`, `create_bounty(creator, bounty_id,
  issue_ref, amount, token)` (locks funds against a bounty ID),
  `submit_claim(bounty_id, claimant, proof_ref)` (records a claim, no fund
  movement), and `release(admin, bounty_id)` (admin-gated, one-time payout
  of locked funds to the claimant). Getters: `get_bounty`, `get_claim`.
- **By:** ALLEN-AYODEJI
- **Closes:** N/A (predates the issue tracker)
- **Status:** Deployed and verified on Stellar testnet. *Superseded by the
  hardened v2 router below.*
  - Contract ID: `CBFLUY6NB3JOQDEUOF2JG5TL3UD7GCQ7APUJ6EHNNSUJLUELHSGIG675`
  - A full `create_bounty` → `submit_claim` → `release` cycle was run
    against the live testnet contract using the native XLM Stellar Asset
    Contract as the test token, and confirmed via `get_bounty` / `get_claim`
    reads and on-chain transfer events.

### typosquat scanner — popular-package reference data + loader

- **What:** Groundwork for the typosquat detection scanner. Adds
  `data/popular-packages/npm-top1000.json` and
  `data/popular-packages/pypi-top1000.json` — each a JSON array of ~1000
  widely-used package names for that registry (a hand-curated representative
  snapshot, not a live-fetched ranking). Introduces the crate's off-chain
  Rust side: a root `deprader` library crate with
  `src/typosquat/dataset.rs`, exposing `load_popular_packages()`, which
  embeds both lists at compile time and returns them as one combined
  `Vec<String>` (npm first, then PyPI). No scoring logic yet — just the data
  and the loader. Covered by a unit test asserting the combined list is
  non-empty and contains known packages (`react`, `requests`).
- **By:** ALLEN-AYODEJI
- **Closes:** N/A (prep for the typosquat scanner module)
- **Status:** Merged. `cargo test --workspace --locked` passes.

### typosquat scanner — edit-distance primitives

- **What:** Adds `src/typosquat/distance.rs`. `levenshtein_distance(a, b)`
  computes standard Levenshtein edit distance (insert / delete / substitute,
  no transposition op) over Unicode scalar values with the two-row DP table,
  no external crate. `nearest_match(candidate, dataset)` returns the closest
  name in a reference slice and its distance (earliest entry wins ties;
  empty slice yields `(String::new(), usize::MAX)`). Wired into
  `src/typosquat/mod.rs`. Still no scoring/thresholding — callers decide
  what distance counts as suspicious.
- **Tests:** identical strings → 0; empty-operand cases; one-character-off
  typosquats (`expres`/`expresss` vs `express`, a single substitution) → 1;
  transposition → 2; `nearest_match` against the loaded popular-package
  dataset resolves deliberately misspelled `expresss` → `express` and
  `requezts` → `requests`, both at distance 1; exact hit → distance 0;
  empty-dataset sentinel.
- **By:** ALLEN-AYODEJI
- **Closes:** N/A (prep for the typosquat scanner module)
- **Status:** Merged. `cargo test --workspace --locked` passes.

### typosquat scanner — homoglyph (lookalike-character) signal

- **What:** Adds `src/typosquat/homoglyph.rs`, a second detection signal that
  sits alongside the edit-distance one and shares no code with it.
  `normalize_homoglyphs(candidate)` rewrites a name by mapping known Unicode
  lookalike characters back to their Latin equivalents — a hand-picked,
  deliberately conservative subset of the Unicode `confusables.txt` (UTR #39):
  Cyrillic, Greek and Armenian letters plus a couple of Latin-extended/IPA
  forms, and the entire full-width ASCII block (`U+FF01..=U+FF5E`) handled as a
  contiguous range. `homoglyph_impersonation(candidate, dataset)` returns
  `Some(name)` only when a substitution actually happened *and* the normalised
  form exactly matches an entry in `dataset` (in practice the combined list
  from `load_popular_packages()`); a correctly-spelled popular package
  normalises to itself and is never flagged. Wired into `src/typosquat/mod.rs`.
  Still no scoring/thresholding — this is a boolean signal for the
  (not-yet-written) scanner to weigh.
- **Tests:** `normalize_homoglyphs` is identity for plain ASCII (incl. a
  scoped `@angular/core` name and the empty string); Cyrillic `а` (U+0430) and
  `о` (U+043E) in `react`/`lodash` → Latin; full-width `ｅｘｐｒｅｓｓ` → `express`;
  Greek omicron (U+03BF) in `commander` → `o`. `homoglyph_impersonation`
  against the loaded popular-package dataset flags a Cyrillic-`а` `react`
  (npm), a full-width `requests` (PyPI) and a Greek-omicron `lodash` (npm);
  returns `None` for a correctly-spelled `react` and for a homoglyph name
  (`reаctxyzzy`) whose normalised form is not a popular package.
- **By:** ALLEN-AYODEJI
- **Closes:** N/A (prep for the typosquat scanner module)
- **Status:** Merged. `cargo test --workspace --locked` passes.

### Testnet read client — get_bounty / get_claim by ID

- **What:** Adds `client/`, a minimal read-only Node/TS client for
  `deprader_router` on Stellar testnet (contract ID
  `CBFLUY6NB3JOQDEUOF2JG5TL3UD7GCQ7APUJ6EHNNSUJLUELHSGIG675`). Uses
  `@stellar/stellar-sdk`'s `rpc.Server.queryContract` to simulate
  `get_bounty(bounty_id)` and `get_claim(bounty_id)` calls against
  `https://soroban-testnet.stellar.org` and print the decoded result — no
  transactions are submitted and no funded account is required, since both
  contract methods are pure reads. CLI: `npm start -- --bounty-id <id>
  --claim-id <id>` (both default to `1`). An ID with no matching bounty or
  claim prints a plain "not found" line rather than erroring, matching the
  contract's `Option<Bounty>` / `Option<Claim>` return type. Kept as a
  standalone `client/` package (its own `package.json`/`tsconfig.json`,
  outside the Cargo workspace) since it's plain TypeScript, not a Soroban
  contract.
- **Verified:** Run against the live contract with the default IDs, which
  resolve to the real `create_bounty` → `submit_claim` → `release` cycle
  recorded above — `get_bounty(1)` returned the stored bounty (amount
  `50000000`, `issue_ref: "org/repo#123"`, `released: true`) and
  `get_claim(1)` returned the stored claim (`proof_ref:
  "org/repo#123-pr-456"`). Also verified an unknown ID (`999`) prints the
  not-found path for both calls instead of throwing.
- **By:** ALLEN-AYODEJI
- **Closes:** #6
- **Status:** Merged. No UI yet, per the issue scope — read-only CLI output
  only.

### CVE matching — OSV API client

- **What:** Adds `src/cve/osv_client.rs`, the first piece of the CVE
  matching scanner class. `query_osv(package_name, ecosystem, version)`
  queries the [OSV.dev](https://osv.dev) API (`POST /v1/query`) for a package
  pinned to an exact version and parses the response into typed structs
  (`OsvQueryResponse` → `Vec<OsvVulnerability>`, with nested `OsvAffected` /
  `OsvRange` / `OsvEvent` / `OsvSeverity` / `OsvReference`) mirroring the
  relevant parts of the [OSV schema](https://ossf.github.io/osv-schema/). The
  `ecosystem` argument is a typed `Ecosystem` enum (`Npm`, `PyPI`,
  `CratesIo`) rather than a free string, mapped to the exact ecosystem names
  OSV expects. No filtering, deduplication, or scoring — an empty `vulns`
  list just means OSV has no known match for that exact version; turning raw
  matches into scored findings is later work. Uses `reqwest` (blocking
  client) for HTTP and `serde`/`serde_json` for (de)serialization — new
  dependencies for the crate, added to `Cargo.toml`.
- **Tests:** `query_osv` against the real OSV API for `lodash@4.17.4` (npm),
  a known-vulnerable pair predating the fix for GHSA-29mw-wpgm-hmr9 /
  CVE-2020-28500 (ReDoS in `trim`/`toNumber`/`trimEnd`, fixed in 4.17.21) —
  asserts the response is non-empty and includes that CVE alias. The test
  skips (rather than fails) if the request itself can't complete, since that
  reflects the runner's network access rather than a bug in the client. Also
  a plain unit test that `Ecosystem::as_osv_str()` matches OSV's documented
  ecosystem names for `npm`/`PyPI`/`crates.io`.
- **By:** MamaLove360 ([#10](https://github.com/DepRadarOrg/DepRadar/pull/10))
- **Closes:** #5
- **Status:** Merged. `cargo test --workspace --locked` passes, including a
  live network call to `api.osv.dev` confirming the known-vulnerable test
  case.

### typosquat scanner — combined scoring API + CLI

- **What:** Adds `src/typosquat/scanner.rs`, the single entry point that ties
  the two independent typosquat signals together:
  `score_package(name: &str) -> ScanResult` loads
  [`load_popular_packages()`](src/typosquat/dataset.rs) internally, runs
  `nearest_match` ([`distance.rs`](src/typosquat/distance.rs)) and
  `homoglyph_impersonation` ([`homoglyph.rs`](src/typosquat/homoglyph.rs))
  against it, and returns a `ScanResult` recording which signal(s) fired and
  why. A `distance_match` only counts when the nearest name is within
  `DISTANCE_THRESHOLD` (2) edits and isn't an exact match (distance 0 is the
  real package, not a typosquat); `homoglyph_match` is whatever
  `homoglyph_impersonation` reports. `ScanResult::is_likely_typosquat()` is
  true if either fired, and either, both, or neither can fire independently
  since the two signals still share no logic. `ScanResult` implements
  `Display` for human-readable output (used by the CLI below). Also adds
  `src/bin/scan.rs`, a small binary (`cargo run --bin scan -- <package-name>`)
  that scores its one argument and prints the result — the manual-testing
  surface the issue asked for. Wired into `src/typosquat/mod.rs`.
- **Tests:** exact popular-package name (`react`) and a long unrelated
  string are both unflagged; a pure edit-distance typo (`expresss` →
  `express`, distance 1) flags only `distance_match`; a full-width `requests`
  flags only `homoglyph_match` (its raw-codepoint edit distance to any
  dataset entry is 8, well past the threshold); a Cyrillic-`а` `react`
  flags *both* (it's simultaneously one substitution away from `react` and a
  homoglyph impersonation of it) — the concrete case the issue asked for.
  Two more tests check `Display` output for the both-signals and no-signal
  cases.
- **By:** Emeraldson001 ([#11](https://github.com/DepRadarOrg/DepRadar/pull/11))
- **Closes:** #4
- **Status:** Merged. Built on the dataset/distance work (#1, #2) and the
  homoglyph signal (#7).

### Repository health — license, security policy, CI hardening

- **What:** Licenses the project under MIT (`LICENSE`, plus `license` fields
  in both Cargo manifests and `client/package.json`). Adds `SECURITY.md`
  (private disclosure via GitHub advisories, scope covering contract and
  scanner), `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1), a PR template,
  bug/feature issue templates, `CODEOWNERS`, and Dependabot for Cargo, npm
  and GitHub Actions. CI now runs `cargo fmt --check`, `cargo clippy -D
  warnings`, the test suite, a `wasm32v1-none` release build of the
  contract, and the client typecheck as separate jobs; a new weekly
  `Dependency Audit` workflow runs `cargo audit` and `npm audit`. Fixes the
  one existing clippy lint (`unnecessary_map_or` in `distance.rs`) and
  applies `cargo fmt`. Bumps the client's `@stellar/stellar-sdk` lock to
  17.2.1 to clear high-severity advisories in its transitive `axios`
  dependency. README links now point at `DepRadarOrg/DepRadar`, and the
  contributor attributions in this file are corrected to match the merged
  PRs.
- **Verified:** `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test
  --workspace --locked` (33 tests) and the wasm build all pass locally;
  `npm audit --omit=dev` reports 0 vulnerabilities; the client still reads
  `get_bounty(1)` / `get_claim(1)` correctly from the live testnet contract
  on the upgraded SDK.
- **By:** ALLEN-AYODEJI
- **Closes:** N/A (maintenance)
- **Status:** Merged.

### deprader_router v2 — claim review, refunds, events, TTL, admin rotation

- **What:** Reworks `contracts/deprader_router` to close the gaps in the v1
  design, where the first claim on a bounty was the only one that could
  ever be paid (a junk claim could block the real finder), unclaimed funds
  had no way out, and nothing was observable without polling.
  - **Multiple claims per bounty.** `submit_claim` returns a `claim_id`;
    any number of addresses can claim (one each, capped at
    `MAX_CLAIMS_PER_BOUNTY` = 50). Claims carry a `Pending` / `Rejected` /
    `Accepted` status and a `submitted_at` timestamp.
  - **Admin review.** New `reject_claim(bounty_id, claim_id)`;
    `release(bounty_id, claim_id)` now pays a *specific* claim. Both check
    the stored admin's auth directly instead of taking an `admin` argument.
  - **Deadlines and refunds.** `create_bounty` takes a `deadline` (max one
    year out); claims close at the deadline. New `cancel_bounty` refunds
    the creator — the admin any time, the creator once the deadline has
    passed with no pending claims or a 7-day review window after it has
    elapsed.
  - **Contract-assigned bounty IDs** (`create_bounty` returns the ID;
    `bounty_count` getter), removing caller-chosen ID collisions.
  - **Events** for every state change: `created`, `claimed`, `rejected`,
    `released`, `cancelled`, `admin_set`, `admin_prp`.
  - **Storage TTL management** — instance and persistent entries are
    extended on every write, so live bounties aren't archived mid-review.
  - **Two-step admin rotation** — `propose_admin` / `accept_admin`, plus
    `get_admin` / `get_pending_admin`.
  - Input validation for `issue_ref` / `proof_ref` (non-empty, ≤ 256 bytes);
    `create_bounty` requires an initialized contract; state is written
    before every token transfer.
- **Tests:** 21 contract tests (up from 7) covering initialization, ID
  assignment, input validation, auth (checked via `env.auths()` and by
  dropping all mocked auths), multi-claim, duplicate/late/over-cap claims,
  the junk-claim-first scenario, release during the review window, every
  cancel path and timing boundary, and admin rotation. Snapshots
  regenerated.
- **Deployed:** testnet contract
  [`CCPRQANY467KKE4ROKCNW6TH3LY3EPVFRXIXJ5V4VCYHLJKWZPOZYQJQ`](https://stellar.expert/explorer/testnet/contract/CCPRQANY467KKE4ROKCNW6TH3LY3EPVFRXIXJ5V4VCYHLJKWZPOZYQJQ)
  with a fresh admin identity (replacing the v1 admin exposed on-chain).
  Verified live with native XLM:
  - Bounty 1: `create_bounty` (5 XLM) → a junk claim (#0) then a real
    claim (#1) → `reject_claim(1, 0)` → `release(1, 1)` paid the real
    claimant
    ([tx](https://stellar.expert/explorer/testnet/tx/69538e183dab1799c92be7231e562fcbca47bd32e0247a44f0cddfbeaee84cb0)).
  - Bounty 2: `create_bounty` (2 XLM) → claim → a non-admin `release`
    attempt failed authorization → admin `cancel_bounty` refunded the
    creator
    ([tx](https://stellar.expert/explorer/testnet/tx/39b406a4eeda3d879c0b45ff2a8eae388838eda9bc185891f6fc47eca1b31912)).
- **Client:** `client/` points at the v2 contract and now prints a bounty
  plus every claim on it (`get_claim(bounty_id, claim_id)`).
- **Breaking:** `create_bounty`, `submit_claim`, `get_claim` and `release`
  signatures changed; `Bounty.released: bool` became `status`.
- **By:** ALLEN-AYODEJI
- **Closes:** N/A (contract hardening)
- **Status:** Merged.

### Web dashboard — read-only bounty explorer

- **What:** Adds `frontend/`, a React 19 + TypeScript + Vite app that reads
  the v2 router on testnet with `@stellar/stellar-sdk` (simulated
  `queryContract` calls, no wallet). It has two routes: `/` lists every
  bounty, newest first, with reward, status, claim count, deadline and
  creator, plus total/open/paid-out stats. `/bounty/:id` shows the bounty's
  facts and every claim with its Pending/Rejected/Accepted status.
  - All contract reads live in `lib/contract.ts`, with `parseBounty` /
    `parseClaim` mapping contract structs to typed camelCase objects.
  - Amount, address and date helpers are in `lib/format.ts`.
  - `useAsync` handles loading/error/retry and ignores stale responses.
  - Styling uses CSS design tokens, dark by default with light following the
    OS. Hash routing means the build works on any static host.
  - The network and contract can be overridden via `.env.local`.
- **Tests:** 16 Vitest tests covering amount formatting (decimals,
  grouping, negatives), address shortening, relative time, contract struct
  parsing (both enum encodings, plus the rejection of unknown shapes), and
  component rendering for the status badge and the error state with retry.
- **CI:** a new `frontend` job runs `oxlint --deny-warnings`, typecheck,
  tests and the build. Dependabot and the weekly npm audit cover
  `frontend/`.
- **Verified:** run locally against the live contract. The list shows both
  testnet bounties, and bounty #1's detail page shows the rejected junk
  claim and the accepted real claim.
- **By:** ALLEN-AYODEJI
- **Closes:** N/A (scaffold for the dashboard issue track)
- **Status:** Merged.

### Web dashboard — visual redesign

- **What:** Restyles `frontend/` around the DepRadar brand.
  - **Home:** a hero with an animated radar in which every blip is a real
    bounty (colored by status, and each one links to its bounty), plus live
    counters for XLM paid out, XLM locked, bounties funded and claims
    submitted.
  - **Bounty board:** cards with an ecosystem tag, a large reward, a status
    accent and the deadline, with All/Open/Paid/Refunded filter chips stored
    in the URL.
  - **Bounty page:** a hero header with the reward, a visual timeline
    (funded → each claim with its verdict → claims close → paid or
    refunded), and a details panel with a copyable finding reference.
  - **Visual system:** dark navy/teal tokens taken from `assets/banner.svg`;
    Space Grotesk, Inter and JetBrains Mono, self-hosted; skeleton loaders,
    address identicons, a radar favicon, and pages that scroll to the top
    on navigation. All animation is disabled under
    `prefers-reduced-motion`.
- **Tests:** 21 frontend tests, up from 16. The new `lib/insights.ts`
  logic (board stats and timeline ordering) is unit-tested.
- **Verified:** checked in the browser at 1280px and 375px against the live
  testnet contract.
- **By:** ALLEN-AYODEJI
- **Closes:** N/A
- **Status:** Merged.

### deprader_router contract — per-token minimum bounty amount

- **What:** Adds `DataKey::MinAmount(Address)` and `Error::AmountTooSmall = 20` to `contracts/deprader_router/src/lib.rs`, along with admin-gated `set_min_amount(token, min_amount)` and public `get_min_amount(token)` (default `0`). `create_bounty` rejects amounts below the configured token minimum with `Error::AmountTooSmall`.
- **Tests:** Contract unit test in `contracts/deprader_router/src/test.rs` covering default `0`, negative `min_amount` rejection, admin auth and event emission, and `create_bounty` below (`99`), at (`100`), and above (`150`) the minimum as well as independent per-token defaults. Snapshots updated and `wasm32v1-none` build verified.
- **By:** area523ufo
- **Closes:** #172
- **Status:** Ready for review.

