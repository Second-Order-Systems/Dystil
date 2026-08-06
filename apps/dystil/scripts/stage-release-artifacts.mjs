import { copyFile, mkdir, readdir } from "node:fs/promises";
import path from "node:path";

function parseArgs(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith("--") || value === undefined) {
      throw new Error(`Invalid argument list near ${key ?? "end of arguments"}`);
    }
    values[key.slice(2)] = value;
  }
  return values;
}

async function filesBelow(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...await filesBelow(entryPath));
    else if (entry.isFile()) files.push(entryPath);
  }
  return files;
}

const args = parseArgs(process.argv.slice(2));
for (const required of ["bundle-dir", "output", "os", "arch", "version"]) {
  if (!args[required]) throw new Error(`Missing --${required}`);
}

const allowedExtensions = {
  windows: [".exe"],
  macos: [".dmg"],
  linux: [".AppImage", ".deb"],
};
const extensions = allowedExtensions[args.os];
if (!extensions) throw new Error(`Unsupported OS: ${args.os}`);
if (!/^\d+\.\d+\.\d+$/.test(args.version)) {
  throw new Error(`Invalid release version: ${args.version}`);
}

const bundleFiles = await filesBelow(path.resolve(args["bundle-dir"]));
// The Rust target directory is intentionally cached between releases, so it
// can contain installers produced for earlier app versions. Only stage files
// belonging to the version requested by this workflow invocation.
const versionedPrefix = `Dystil_${args.version}`;
const selected = bundleFiles.filter((file) =>
  path.basename(file).startsWith(versionedPrefix)
  && extensions.some((extension) => file.endsWith(extension)),
);

if (selected.length !== extensions.length) {
  throw new Error(
    `Expected ${extensions.length} installable ${args.os} artifact(s), found ${selected.length}:\n${selected.join("\n")}`,
  );
}
for (const extension of extensions) {
  if (selected.filter((file) => file.endsWith(extension)).length !== 1) {
    throw new Error(`Expected exactly one ${extension} artifact`);
  }
}

const destinationDirectory = path.resolve(
  args.output,
  args.os,
  "individual",
);
await mkdir(destinationDirectory, { recursive: true });

for (const source of selected) {
  const extension = extensions.find((candidate) => source.endsWith(candidate));
  const filename = `Dystil_${args.version}-${args.arch}-setup-individual${extension}`;
  const destination = path.join(destinationDirectory, filename);
  await copyFile(source, destination);
  console.log(`${source} -> ${destination}`);
}
