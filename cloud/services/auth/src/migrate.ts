import { auth } from "./auth.ts";

async function main() {
  const context = await auth.$context;
  await context.runMigrations();
  console.log("[auth] Better Auth migrations applied");
}

main().catch((error) => {
  console.error("[auth] Better Auth migrations failed", error);
  process.exit(1);
});
