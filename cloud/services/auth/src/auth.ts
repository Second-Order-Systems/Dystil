import { betterAuth } from "better-auth";
import { bearer, jwt } from "better-auth/plugins";
import { tauri } from "./tauri-plugin.ts";
import { Pool } from "pg";

import { sendAuthEmail, verificationEmailHtml, resetPasswordEmailHtml } from "./email.ts";

const databaseUrl = process.env.DATABASE_URL;
const betterAuthUrl = process.env.BETTER_AUTH_URL;
const betterAuthSecret = process.env.BETTER_AUTH_SECRET;
if (!databaseUrl) {
  throw new Error("DATABASE_URL is required");
}

if (!betterAuthUrl) {
  throw new Error("BETTER_AUTH_URL is required");
}

if (!betterAuthSecret) {
  throw new Error("BETTER_AUTH_SECRET is required");
}

export function createAuth(options: {
  basePath: string;
  workspaceOnly: boolean;
}) {
  const google = options.workspaceOnly
    ? {
        clientId: process.env.GOOGLE_CLIENT_ID!,
        clientSecret: process.env.GOOGLE_CLIENT_SECRET!,
        hd: "*" as const,
      }
    : {
        clientId: process.env.GOOGLE_CLIENT_ID!,
        clientSecret: process.env.GOOGLE_CLIENT_SECRET!,
      };

  return betterAuth({
  appName: options.workspaceOnly ? "Dystil Workspace" : "Dystil Individual",
  baseURL: betterAuthUrl,
  basePath: options.basePath,
  secret: betterAuthSecret,
  database: new Pool({
    connectionString: databaseUrl,
  }),
  oauthConfig: {
    skipStateCookieCheck: true,
  },
  socialProviders: {
    google: {
      ...google,
    },
    github: {
      clientId: process.env.GITHUB_CLIENT_ID!,
      clientSecret: process.env.GITHUB_CLIENT_SECRET!,
    },
  },
  trustedOrigins: [
    "http://localhost:1420",
    "http://localhost:5173",
    "http://localhost:3000",
    "http://*.localhost:3000",
    "tauri://localhost",
    "http://tauri.localhost",
    "dystil:/",
    betterAuthUrl!,
    "https://dystil.2os.ai",
    "https://*.dystil.2os.ai",
    // Tenant dashboards are served from <tenant>.2os.ai.
    "https://*.2os.ai",
  ],
  emailVerification: {
    sendVerificationEmail: async ({ user, url }) => {
      await sendAuthEmail({
        to: user.email,
        subject: "Verify your Dystil email",
        text: `Verify your Dystil email by opening this link: ${url}`,
        html: await verificationEmailHtml(url, user.email),
      });
    },
  },
  emailAndPassword: {
    enabled: true,
    requireEmailVerification: false,
    revokeSessionsOnPasswordReset: true,
    sendResetPassword: async ({ user, url }) => {
      await sendAuthEmail({
        to: user.email,
        subject: "Reset your Dystil password",
        text: `Reset your Dystil password by opening this link: ${url}`,
        html: await resetPasswordEmailHtml(url, user.email),
      });
    },
  },
  plugins: [
    bearer(),
    jwt(),
    tauri({
      scheme: "dystil",
      callbackURL: "/",
      debugLogs: true,
    }),
  ],
  });
}

// Workspace authentication is the default flow.
export const auth = createAuth({
  basePath: "/api/auth",
  workspaceOnly: true,
});

export const individualAuth = createAuth({
  basePath: "/api/auth/individual",
  workspaceOnly: false,
});
