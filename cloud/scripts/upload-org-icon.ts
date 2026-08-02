import { readFileSync } from "fs";
import { extname } from "path";
import { resolve } from "path";
import { AwsClient } from "aws4fetch";
import sharp from "sharp";

interface StorageEnv {
  accountId: string;
  publicBucket: string;
  accessKeyId: string;
  secretAccessKey: string;
  publicBaseUrl: string;
}

function loadFromEnvFile(name: string): string | undefined {
  const scriptsDir = import.meta.dirname;
  for (const filename of [".env", ".env.docker"]) {
    const envPath = resolve(scriptsDir, "..", filename);
    try {
      for (const line of readFileSync(envPath, "utf-8").split("\n")) {
        const trimmed = line.trim();
        if (trimmed.startsWith("#") || trimmed.length === 0) continue;
        const m = trimmed.match(new RegExp(`^${name}\\s*=\\s*(.+)$`));
        if (m) return m[1]!.trim();
      }
    } catch {
      // file doesn't exist or can't be read
    }
  }
  return undefined;
}

function loadStorageEnv(): StorageEnv | null {
  const resolve = (name: string): string | undefined =>
    process.env[name] || loadFromEnvFile(name);

  const accountId = resolve("R2_ACCOUNT_ID");
  const publicBucket = resolve("R2_PUBLIC_BUCKET");
  const accessKeyId = resolve("R2_ACCESS_KEY_ID");
  const secretAccessKey = resolve("R2_SECRET_ACCESS_KEY");

  if (!accountId || !publicBucket || !accessKeyId || !secretAccessKey) {
    const missing = [
      !accountId && "R2_ACCOUNT_ID",
      !publicBucket && "R2_PUBLIC_BUCKET",
      !accessKeyId && "R2_ACCESS_KEY_ID",
      !secretAccessKey && "R2_SECRET_ACCESS_KEY",
    ]
      .filter(Boolean)
      .join(", ");
    console.warn(`warning: missing R2 env vars (${missing}), skipping org icon upload`);
    return null;
  }

  const publicBaseUrl =
    resolve("R2_PUBLIC_BASE_URL") ||
    `https://pub.r2.dev/org-icons`; // fallback: placeholder dev URL

  return { accountId, publicBucket, accessKeyId, secretAccessKey, publicBaseUrl };
}

function detectMimeFromExtension(filePath: string): string | null {
  const ext = extname(filePath).toLowerCase();
  switch (ext) {
    case ".png":
      return "image/png";
    case ".jpg":
    case ".jpeg":
      return "image/jpeg";
    default:
      return null;
  }
}

export async function uploadOrgIcon(
  orgId: string,
  orgSlug: string,
  iconPath: string,
): Promise<void> {
  const storage = loadStorageEnv();
  if (!storage) return;

  const mime = detectMimeFromExtension(iconPath);
  if (!mime) {
    console.warn(
      `warning: unsupported icon file type "${extname(iconPath)}", expected .png or .jpg`,
    );
    return;
  }

  let pngBuffer: Buffer;
  try {
    pngBuffer = await sharp(iconPath).png().toBuffer();
  } catch (err) {
    console.warn(`warning: could not convert icon to PNG: ${err instanceof Error ? err.message : err}`);
    return;
  }

  const objectKey = `org/${orgId}/${orgSlug}.png`;
  const r2Endpoint = `https://${storage.accountId}.r2.cloudflarestorage.com`;
  const url = `${r2Endpoint}/${storage.publicBucket}/${objectKey}`;

  const client = new AwsClient({
    accessKeyId: storage.accessKeyId,
    secretAccessKey: storage.secretAccessKey,
    region: "auto",
    service: "s3",
  });

  try {
    const response = await client.fetch(url, {
      method: "PUT",
      body: pngBuffer,
      headers: {
        "Content-Type": "image/png",
      },
    });

    if (!response.ok) {
      const text = await response.text().catch(() => "<could not read body>");
      console.warn(
        `warning: icon upload failed (${response.status}): ${text}`,
      );
      return;
    }

    console.log(
      `Org icon uploaded (${pngBuffer.length} bytes): ${storage.publicBaseUrl}/${objectKey}`,
    );
  } catch (err) {
    console.warn(`warning: icon upload failed: ${err instanceof Error ? err.message : err}`);
  }
}

if (import.meta.main) {
  const [orgId, orgSlug, iconPath] = process.argv.slice(2);
  if (!orgId || !orgSlug || !iconPath) {
    console.error("Usage: bun run scripts/upload-org-icon.ts <org-id> <org-slug> <icon-path>");
    process.exit(1);
  }
  uploadOrgIcon(orgId, orgSlug, iconPath).catch((err) => {
    console.error("Upload failed:", err);
    process.exit(1);
  });
}
