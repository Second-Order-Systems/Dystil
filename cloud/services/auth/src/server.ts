import { Hono } from "hono";
import { cors } from "hono/cors";

import { auth, individualAuth } from "./auth.ts";

const app = new Hono();

const exactCorsOrigins = new Set(
  [
    "http://localhost:1420",
    "http://localhost:5173",
    "tauri://localhost",
    "http://tauri.localhost",
    process.env.BETTER_AUTH_URL,
  ].filter((value): value is string => Boolean(value)),
);

function isAllowedCorsOrigin(origin: string): boolean {
  if (exactCorsOrigins.has(origin)) return true;

  try {
    const url = new URL(origin);
    return url.protocol === "https:" && url.hostname.endsWith(".2os.ai");
  } catch {
    return false;
  }
}

app.use(
  "*",
  cors({
    origin: (origin) => {
      if (!origin) return undefined;
      return isAllowedCorsOrigin(origin) ? origin : undefined;
    },
    credentials: true,
    allowHeaders: ["Content-Type", "Authorization", "Platform"],
    allowMethods: ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"],
    exposeHeaders: ["set-auth-token", "set-auth-jwt"],
  }),
);

app.get("/health", (c) => {
  return c.json({ ok: true, service: "better-auth" });
});

app.on(["GET", "POST"], "/api/auth/individual/*", (c) => {
  return individualAuth.handler(c.req.raw);
});

// Keep the more specific route before /api/auth/*.
app.on(["GET", "POST"], "/api/auth/*", (c) => {
  return auth.handler(c.req.raw);
});

const port = Number(process.env.AUTH_PORT ?? 3001);

Bun.serve({
  fetch: app.fetch,
  hostname: process.env.AUTH_HOST ?? "127.0.0.1",
  port,
});

console.log(`Better Auth service listening on port ${port}`);
