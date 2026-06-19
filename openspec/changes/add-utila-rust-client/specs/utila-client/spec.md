# utila-client

## ADDED Requirements

### Requirement: Full v2 API coverage via a curated facade
The crate SHALL expose every operation in the Utila v2 OpenAPI specification. The
progenitor-generated client (from the vendored spec) is the transport for all 42
operations, and the hand-written grouped facade SHALL expose each of them as an ergonomic
async method that takes typed-ID newtypes and returns a **curated facade type** —
hand-mapped structs/enums — rather than a raw generated `types::V2*` struct.

#### Scenario: Generated client covers all operations
- **WHEN** the client is generated from `openapi/utila.v2.json`
- **THEN** all 42 v2 operations across the 7 resource groups (vaults, wallets, balances,
  transactions, assets, blockchains/networks, address book) are callable
- **AND** the generated code compiles with no warnings

#### Scenario: Facade exposes a curated method for every operation
- **WHEN** a caller uses the grouped facade (e.g. `client.wallets().list(...)`)
- **THEN** each of the 42 operations is reachable through a facade method taking typed IDs
- **AND** the method returns a curated facade type, never a raw `types::V2*` struct

#### Scenario: AIP custom-method paths are correct
- **WHEN** a custom-method operation such as `transactions:initiate` is invoked
- **THEN** the request targets `/v2/vaults/{vault_id}/transactions:initiate` with the colon verb preserved

### Requirement: Service-account JWT authentication
The client SHALL authenticate every request with a self-signed RS256 JWT bearing
claims `sub` (service-account email), `aud` `https://api.utila.io/`, and `exp`
(issued time + 1 hour), signed with the service-account RSA key. No token-exchange
endpoint is called.

#### Scenario: Request carries a freshly minted bearer token
- **WHEN** any API method is called with a valid service-account credential
- **THEN** the request includes `Authorization: Bearer <jwt>`
- **AND** the JWT `aud` is exactly `https://api.utila.io/` (with trailing slash)

#### Scenario: Cached token is reused and refreshed before expiry
- **WHEN** multiple requests are made within the token lifetime
- **THEN** a single cached token is reused
- **AND** a new token is minted once the cached token is within 5 minutes of expiry

#### Scenario: Signing failure aborts the request
- **WHEN** token minting or refresh fails (e.g. invalid key or KMS error)
- **THEN** the request returns an error before any network call is made

#### Scenario: Pluggable signer
- **WHEN** the client is configured with a local RSA PEM key or a KMS key URL
- **THEN** signing is performed by the corresponding `Signer` implementation
- **AND** the rest of the auth flow is identical

### Requirement: Configuration via builder
The client SHALL be constructed via an explicit builder taking the service-account email
and an already-validated signer. As a library it SHALL NOT read environment variables
itself — env/secret handling is the caller's concern. The builder SHALL allow configuring
request and connect timeouts.

#### Scenario: Construct via builder
- **WHEN** `.credential(account, signer)` is supplied to `UtilaClient::builder()` and `.build()` is called
- **THEN** a ready client is returned

#### Scenario: Timeouts are configurable
- **WHEN** `.timeout(d)` and/or `.connect_timeout(d)` are set on the builder
- **THEN** the underlying HTTP client enforces those timeouts on every request

### Requirement: Typed API errors
The client SHALL surface non-success responses as a typed error parsed from the
gRPC status envelope.

#### Scenario: Error response is parsed
- **WHEN** the API returns a non-2xx response with a `GooglerpcStatus` body
- **THEN** the caller receives a `UtilaError::Api` exposing `code` and `message`

### Requirement: Exact monetary amounts
The client SHALL represent monetary amounts as exact integer base units and provide
a lossless decimal projection given an asset's decimals.

#### Scenario: Round-trip an amount without loss
- **WHEN** an amount string of integer base units is parsed and re-serialized
- **THEN** the output equals the input exactly

#### Scenario: Project to a human-readable decimal
- **WHEN** `to_decimal(decimals)` is called with the asset's decimals
- **THEN** the returned decimal equals base units scaled by `10^-decimals`

### Requirement: Streaming pagination
The client SHALL provide an async stream that walks paginated List endpoints via
`pageToken` until exhausted, in addition to raw single-page access.

#### Scenario: Stream yields all pages
- **WHEN** a List endpoint with multiple pages is streamed
- **THEN** items from every page are yielded in order until `nextPageToken` is empty

#### Scenario: Mid-stream error is surfaced
- **WHEN** a page request fails partway through streaming
- **THEN** the stream yields an error item rather than silently terminating

### Requirement: Type-safe transaction initiation
The client SHALL model the transaction `details` input as an enum with one variant per
message type, making it impossible to specify zero or multiple detail types.

#### Scenario: One detail variant is sent
- **WHEN** a transaction is initiated with a `TransactionDetails` variant (e.g. `AssetTransfer`)
- **THEN** the request body sets exactly that one detail field and no other

#### Scenario: Optional modifiers are independent
- **WHEN** optional fields such as priority or note are supplied to `initiate`
- **THEN** each is sent only if set, independently of the others

### Requirement: Idempotent transaction initiation
The client SHALL attach a UUID `requestId` to each initiation and allow the caller to
override it so a retry is deduplicated by the server.

#### Scenario: Auto-generated request id is surfaced
- **WHEN** a transaction is initiated without an explicit request id
- **THEN** a UUID `requestId` is sent and made available to the caller

#### Scenario: Caller overrides the request id for a safe retry
- **WHEN** the caller supplies a request id and repeats the call
- **THEN** the same `requestId` is sent so the server can deduplicate

### Requirement: External retry
The client SHALL NOT retry automatically (each call issues exactly one request). Instead it
SHALL expose `UtilaError::is_retryable()` classifying transient transport and server-side
errors, so a caller can wrap a whole operation with a retry crate (e.g. `backon`) and gate
it on that predicate. Rationale: the generated client owns a concrete `reqwest::Client`
(no middleware seam) and operation class cannot be derived from HTTP method (`queryBalances`
is a read over POST), so the retry policy is the caller's to choose.

#### Scenario: Transient errors are classified retryable
- **WHEN** a request fails with a connect/timeout transport error or a 5xx/429/UNAVAILABLE status
- **THEN** `is_retryable()` returns true

#### Scenario: Client errors are not retryable
- **WHEN** a request fails with a client-side error (not-found, bad request, auth, config)
- **THEN** `is_retryable()` returns false

### Requirement: Webhook signature verification
The client SHALL verify inbound webhook signatures (RSA-4096 / SHA-512 / PSS over the
raw body, `x-utila-signature`) against Utila's public key and expose typed events.

#### Scenario: Authentic event verifies and parses
- **WHEN** a webhook body with a valid signature is verified
- **THEN** verification succeeds and the body parses into a typed `Event`

#### Scenario: Tampered body is rejected
- **WHEN** a webhook body does not match its signature
- **THEN** verification fails and no event is returned

### Requirement: Reproducible generation from a vendored spec
The crate SHALL vendor the OpenAPI spec and commit generated code, with a check
that detects drift between the committed code and a fresh regeneration.

#### Scenario: Drift is detected
- **WHEN** the committed generated code differs from a fresh `xtask gen`
- **THEN** the CI drift check fails
