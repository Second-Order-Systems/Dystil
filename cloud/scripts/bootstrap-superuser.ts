import { Pool } from "pg";
import { scrypt, randomBytes } from "node:crypto";
import { readFileSync } from "fs";
import { resolve } from "path";
import { uploadOrgIcon } from "./upload-org-icon";

interface Args {
  email: string;
  password: string;
  name: string;
  orgName: string;
  orgSlug: string;
  allowedEmailDomains: string[];
  iconPath?: string;
}

function parseArgs(): Args {
  const args = process.argv.slice(2);
  if (args.length < 6) {
    console.error(
      [
        "Usage: bun run scripts/bootstrap-superuser.ts <email> <password> <name>",
        "  <org-name> <org-slug> <allowed-email-domains,comma-separated> [icon-path]",
        "",
        "Example:",
        "  bun run scripts/bootstrap-superuser.ts admin@meridian.com s3cret 'Admin User' \\",
        "    'Meridian Corp' meridian 'meridian.com' ./logo.png",
      ].join("\n"),
    );
    process.exit(1);
  }
  return {
    email: args[0]!.trim().toLowerCase(),
    password: args[1]!,
    name: args[2]!.trim(),
    orgName: args[3]!.trim(),
    orgSlug: args[4]!.trim().toLowerCase(),
    allowedEmailDomains: args[5]!
      .split(",")
      .map((d) => d.trim().toLowerCase())
      .filter(Boolean),
    iconPath: args[6]?.trim() || undefined,
  };
}

function loadDbUrl(): string {
  const envPath = resolve(import.meta.dirname, "..", ".env");
  try {
    for (const line of readFileSync(envPath, "utf-8").split("\n")) {
      const m = line.match(/^DATABASE_URL\s*=\s*(.+)/);
      if (m) return m[1]!.trim();
    }
  } catch {}
  const fromEnv = process.env.DATABASE_URL;
  if (fromEnv) return fromEnv;
  console.error("DATABASE_URL not found in .env or environment.");
  process.exit(1);
}

async function detectColumns(pool: Pool): Promise<Columns> {
  const userRows = await pool.query<{ column_name: string }>(
    `SELECT column_name FROM information_schema.columns WHERE table_name = 'user' ORDER BY ordinal_position`,
  );
  const accountRows = await pool.query<{ column_name: string }>(
    `SELECT column_name FROM information_schema.columns WHERE table_name = 'account' ORDER BY ordinal_position`,
  );

  const userCols = new Set(userRows.rows.map((r) => r.column_name));
  const accountCols = new Set(accountRows.rows.map((r) => r.column_name));

  return {
    userEmail: findCol(userCols, "email"),
    userEmailVerified: findCol(userCols, "email_verified", "emailVerified"),
    userName: findCol(userCols, "name"),
    userCreated: findCol(userCols, "created_at", "createdAt"),
    userUpdated: findCol(userCols, "updated_at", "updatedAt"),
    accountUserId: findCol(accountCols, "user_id", "userId"),
    accountProviderId: findCol(accountCols, "provider_id", "providerId"),
    accountPassword: findCol(accountCols, "password"),
    accountCreated: findCol(accountCols, "created_at", "createdAt"),
    accountUpdated: findCol(accountCols, "updated_at", "updatedAt"),
  };
}

function findCol(cols: Set<string>, ...candidates: string[]): string {
  for (const c of candidates) {
    if (cols.has(c)) return c;
  }
  return candidates[0]!; // best guess
}

interface Columns {
  userEmail: string;
  userEmailVerified: string;
  userName: string;
  userCreated: string;
  userUpdated: string;
  accountUserId: string;
  accountProviderId: string;
  accountPassword: string;
  accountCreated: string;
  accountUpdated: string;
}

async function main() {
  const args = parseArgs();
  const databaseUrl = loadDbUrl();

    console.log(
    `Bootstrapping superuser...
  Email:     ${args.email}
  Name:      ${args.name}
  Org:       ${args.orgName} (${args.orgSlug})
  Domains:   ${args.allowedEmailDomains.join(", ") || "(none)"}${args.iconPath ? `\n  Icon:      ${args.iconPath}` : ""}`,
  );

  const pool = new Pool({ connectionString: databaseUrl, max: 2 });

  try {
    const cols = await detectColumns(pool);
    console.log(`Detected user table columns: email=${cols.userEmail}, verified=${cols.userEmailVerified}`);

    const userId = await ensureBetterAuthUser(pool, args.email, args.password, args.name, cols);
    const appUserId = await ensureAppUser(pool, userId, args.email, args.name);
    const orgId = await ensureOrganization(pool, args.orgName, args.orgSlug, args.allowedEmailDomains);
    await setUserOrg(pool, appUserId, orgId);
    await setSuperuser(pool, orgId, appUserId);

    if (args.iconPath) {
      await uploadOrgIcon(orgId, args.orgSlug, args.iconPath);
    }

    console.log(`\nDone. ${args.email} is superuser of ${args.orgName}.`);
    console.log(`Sign in at: ${args.orgSlug}.dystil.2os.ai`);
    console.log(`Dev:         localhost:3000/?org=${args.orgSlug}`);
  } finally {
    await pool.end();
  }
}

async function ensureBetterAuthUser(
  pool: Pool,
  email: string,
  password: string,
  name: string,
  cols: Columns,
): Promise<string> {
  const existing = await pool.query<{ id: string; verified: boolean }>(
    `SELECT id, "${cols.userEmailVerified}" AS verified FROM "user" WHERE "${cols.userEmail}" = $1`,
    [email],
  );
  let row = existing.rows[0];

  if (row) {
    console.log(`Better Auth user exists: ${row.id}`);
    if (!row.verified) {
      await pool.query(`UPDATE "user" SET "${cols.userEmailVerified}" = true WHERE id = $1`, [row.id]);
      console.log(`Set ${cols.userEmailVerified} = true`);
    }
    await ensureCredentialAccount(pool, row.id, password, cols);
    return row.id;
  }

  const userId = crypto.randomUUID();
  await pool.query(
    `INSERT INTO "user" (id, "${cols.userEmail}", "${cols.userName}", "${cols.userEmailVerified}", "${cols.userCreated}", "${cols.userUpdated}")
     VALUES ($1, $2, $3, true, now(), now())`,
    [userId, email, name],
  );
  console.log(`Better Auth user created: ${userId}`);

  await ensureCredentialAccount(pool, userId, password, cols);
  return userId;
}

async function ensureCredentialAccount(
  pool: Pool,
  userId: string,
  password: string,
  cols: Columns,
) {
  const existing = await pool.query<{ id: string }>(
    `SELECT id FROM account WHERE "${cols.accountUserId}" = $1 AND "${cols.accountProviderId}" = 'credential'`,
    [userId],
  );
  if (existing.rows.length > 0) {
    console.log("Credential account exists");
    return;
  }

  const hashPassword = (password: string): Promise<string> =>
  new Promise((resolve, reject) => {
    const salt = randomBytes(16).toString("hex");
    scrypt(
      password.normalize("NFKC"),
      salt,
      64,
      { N: 16384, r: 16, p: 1, maxmem: 128 * 16384 * 16 * 2 },
      (err, key) => {
        if (err) reject(err);
        else resolve(`${salt}:${key.toString("hex")}`);
      },
    );
  });
  const hashedPassword = await hashPassword(password);
  const id = crypto.randomUUID();
  await pool.query(
    `INSERT INTO account (id, "accountId", "${cols.accountProviderId}", "${cols.accountUserId}", "${cols.accountPassword}", "${cols.accountCreated}", "${cols.accountUpdated}")
     VALUES ($1, $2, 'credential', $3, $4, now(), now())`,
    [id, userId, userId, hashedPassword],
  );
  console.log("Credential account created");
}

async function ensureAppUser(
  pool: Pool,
  userId: string,
  email: string,
  name: string,
): Promise<string> {
  const existing = await pool.query<{ id: string }>(
    `SELECT id FROM app_users WHERE user_id = $1`, [userId],
  );
  let row = existing.rows[0];

  if (row) {
    console.log(`app_users row exists: ${row.id}`);
    await pool.query(
      `UPDATE app_users SET email = $1, display_name = $2, last_seen_at = now() WHERE id = $3`,
      [email, name, row.id],
    );
    return row.id;
  }

  const id = crypto.randomUUID();
  await pool.query(
    `INSERT INTO app_users (id, user_id, email, display_name) VALUES ($1, $2, $3, $4)`,
    [id, userId, email, name],
  );
  console.log(`app_users row created: ${id}`);
  return id;
}

async function ensureOrganization(
  pool: Pool,
  name: string,
  slug: string,
  domains: string[],
): Promise<string> {
  const existing = await pool.query<{ id: string }>(
    `SELECT id FROM organizations WHERE slug = $1`, [slug],
  );
  let row = existing.rows[0];

  if (row) {
    console.log(`Organization exists: ${row.id}`);
    await pool.query(
      `UPDATE organizations SET name = $1, allowed_email_domains = $2 WHERE id = $3`,
      [name, domains, row.id],
    );
    return row.id;
  }

  const id = crypto.randomUUID();
  await pool.query(
    `INSERT INTO organizations (id, name, slug, allowed_email_domains) VALUES ($1, $2, $3, $4)`,
    [id, name, slug, domains],
  );
  console.log(`Organization created: ${id}`);
  return id;
}

async function setUserOrg(pool: Pool, appUserId: string, orgId: string) {
  await pool.query(`UPDATE app_users SET org_id = $1 WHERE id = $2`, [orgId, appUserId]);
  console.log(`app_users.org_id = ${orgId}`);
}

async function setSuperuser(pool: Pool, orgId: string, appUserId: string) {
  await pool.query(`UPDATE organizations SET superuser_user_id = $1 WHERE id = $2`, [appUserId, orgId]);
  console.log(`organizations.superuser_user_id = ${appUserId}`);
}

main().catch((err) => {
  console.error("Bootstrap failed:", err.message);
  process.exit(1);
});