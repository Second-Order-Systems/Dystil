# Work Insights Cloud

Portable cloud ingest for Dystil work-insights.

## Local Development

```bash
cd cloud
cp .env.docker.example .env.docker
# Fill in SMTP, Better Auth secret, and any OAuth provider keys.
docker compose --env-file .env.docker up --build
```

To include the optional memory services:

```bash
docker compose -f docker-compose.yml -f docker-compose.memory.yml \
  --env-file .env.docker up --build
```

To run with lower resource limits:

```bash
docker compose -f docker-compose.yml -f docker-compose.local.yml \
  --env-file .env.docker up --build
```

Combine all three:

```bash
docker compose -f docker-compose.yml -f docker-compose.memory.yml \
  -f docker-compose.local.yml --env-file .env.docker up --build
```

Health checks:

```bash
curl -f http://localhost:8089/health
curl -f http://localhost:8089/health/auth
```

The `dystil-api` service runs both the ingest API (Rust) and the Better Auth
sidecar (Bun) inside a single container. Auth listens on `127.0.0.1:3001`
internally and stays private to the container; the API on `0.0.0.0:8089` is
the only public entrypoint.

Local Docker envs are documented in [`.env.docker.example`](.env.docker.example).

## Azure VM / Production

Production uses a single image:

```bash
# Build the image (context is the repo root)
docker build -f cloud/Dockerfile -t dystil-api .
```

On the VM:

```bash
# Create a private env file from the template
cp cloud/.env.vm.example /opt/dystil/app.env
# Fill in all secrets

# Run via docker compose
CONTAINER_REGISTRY=your-registry.example.com \
ENV_FILE=/opt/dystil/app.env \
docker compose -f cloud/docker-compose.prod.yml up -d
```

Production env rules:

1. `BETTER_AUTH_URL` must be the public HTTPS URL.
2. `AUTH_INTERNAL_URL` should be `http://127.0.0.1:3001` (auth runs locally).
3. `AUTH_HOST` should be `127.0.0.1` — auth is only accessed locally by the API
   in the same container.
4. `WORK_INSIGHTS_DATABASE_URL` and `DATABASE_URL` must point at the
   production Postgres host.
5. Secrets should be stored outside the repo and outside the image.

Bootstrap the first organization + owner before testing `/me`:

```bash
cd cloud
export WORK_INSIGHTS_DATABASE_URL=postgres://work_insights:work_insights@localhost:55432/work_insights
cargo run -p work-insights-ingest-api --bin bootstrap_org -- \
  --org-name "Acme" \
  --org-slug acme \
  --owner-supabase-user-id <better-auth-user-id> \
  --owner-email founder@acme.com \
  --domain acme.com
```

Alternatively, bootstrap via TypeScript to also create the Better Auth
credential account. Requires Bun and `cd cloud/scripts && bun install`:

```bash
bun run bootstrap-superuser.ts \
  admin@example.com s3cret "Admin User" \
  "Acme" acme "acme.com" \
  ./logo.png   # optional icon (PNG or JPG, converted to PNG)
```

If the optional 7th argument (icon path) is provided, the icon is converted
to PNG and uploaded to the Cloudflare R2 public bucket at
`org/{orgId}/{orgSlug}.png`.

To upload an icon for an existing org without re-bootstrapping:

```bash
bun run upload-org-icon.ts <org-id> <org-slug> <icon-path>
```

Requires R2 env vars: `R2_ACCOUNT_ID`, `R2_PUBLIC_BUCKET`,
`R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, and optionally
`R2_PUBLIC_BASE_URL`.

## Dystil AI Gateway

The ingest API can expose an OpenAI-compatible gateway at `/v1`. Enable it with
one variable in the API's server-side environment:

```bash
DYSTIL_OPENAI_API_KEY=sk-...
```

The compiled-in catalog contains `gpt-5.6-sol`, `gpt-5.6-terra`, and
`gpt-5.6-luna` with their OpenAI standard short-context prices as checked on
2026-08-02. Only these models are returned by `GET /v1/models`, and every
issued key may use all three. Update `default_models` in
`services/ingest-api/src/ai_gateway.rs` when OpenAI changes its recommended
models or prices.

`DYSTIL_OPENAI_BASE_URL` is optional and defaults to
`https://api.openai.com/v1`. The public client endpoint is
`https://coconut.2os.ai/v1`.

Issue or revoke a key from an admin machine with production database access:

```bash
cd cloud
export WORK_INSIGHTS_DATABASE_URL=postgres://...
cargo run -p work-insights-ingest-api --bin ai_key -- \
  issue --email person@example.com --limit-usd 10
cargo run -p work-insights-ingest-api --bin ai_key -- \
  revoke --key-prefix dst_live_abcd1234
```

The raw key is printed only when issued. The database stores its SHA-256 hash.
The gateway checks lifetime spend before each request; a final request may
cross the limit, and later requests receive `429 insufficient_quota`. The
gateway removes client `max_completion_tokens` and `max_tokens` values, so
Dystil adds no output ceiling; the selected model's intrinsic limit still
applies.

### R2 Public Bucket Setup (one-time)

1. Create an R2 bucket (e.g. `dystil-public`) in the Cloudflare dashboard
2. Enable public access: Settings → Public Development URL (for dev)
   or Settings → Custom Domains (for production, e.g. `icons.2os.ai`)
3. Create an R2 API token scoped to the public bucket with
   "Object Read & Write" permission
4. Set the resulting Access Key ID and Secret Access Key as
   `R2_ACCESS_KEY_ID` and `R2_SECRET_ACCESS_KEY` env vars

## Workspace Layout

Deployable processes live under `services/`. Library crates live under
`crates/`.

- `services/ingest-api`
  - public segment and image ingest API
- `crates/work-insights-db`
  - migrations, SQL queries, and DB transactions
- `crates/work-insights-ingest`
  - segment decode, validation, and DB ingest workflow
- `../memory`
  - Python segment-to-episode worker, query API, memory-owned migration
    runner, migrations beyond `memory_segments`, and memory deployment docs

## Deployment Shape

One deployable service that bundles the API and auth sidecar:

- `dystil-api`
  - Rust ingest API (port 8089) + Bun Better Auth sidecar (internal port 3001)

## Compose Files

| File | Purpose |
|------|---------|
| `docker-compose.yml` | Base: postgres + dystil-api |
| `docker-compose.memory.yml` | Optional overlay: memory-api, memory-worker, memory-migrate |
| `docker-compose.local.yml` | Optional overlay: low-resource overrides |
| `docker-compose.prod.yml` | Production: prebuilt image, resource limits, watchtower |

## Synchronous Ingest Flow

The ingest API validates and stores stable local segments on the request path:

```text
local sync -> dystil-api -> Postgres
```

`POST /v1/ingest/segments` verifies the compressed payload hash, validates each
segment revision and its canonical content hash, and atomically upserts the
segments into `memory_segments`. Identical revision retries are idempotent.

## Identity and Onboarding

The API splits authenticated user access from background ingest:

- `GET /me`
- `POST /devices/register`
- `GET /devices`
- `POST /devices/{device_id}/revoke`

The user-facing endpoints above expect `Authorization: Bearer <better-auth-session>`,
mirror the user into `app_users`, resolve org membership, and manage per-device
credentials for ingest.

Background ingest endpoints use `Authorization: Device <device-token>` as
the primary path. The server resolves canonical `org_id`, `app_users.id`, and
`devices.id` from that token before writing ingest rows.

Authenticated report reads under `/v1/reports/me/*` use the Better Auth session
path again and return correct data for newly ingested rows stamped with
canonical app and device ids.
