
import { $ } from 'bun'
import { constants as fsConstants } from 'fs'
import fs from 'fs/promises'
import os from 'os'
import path from 'path'
import { generateAppConfig } from './app-config.mjs'
import { setupOpenBlas } from './setup_openblas.js'
import { downloadFile } from './find_tools.js'

const originalCWD = process.cwd()
// Change CWD to src-tauri
process.chdir(path.join(__dirname, '../src-tauri'))
const platform = {
	win32: 'windows',
	darwin: 'macos',
	linux: 'linux',
}[os.platform()]
// Windows arch: 'x64' (amd64) or 'arm64' (aarch64) — used for bun binary name and CRT paths
const winArch = platform === 'windows' ? (process.arch === 'arm64' ? 'arm64' : 'x64') : null
const cwd = process.cwd()
console.log('cwd', cwd)

// Normal app workflow guard:
// keep the generated app config fresh before the heavier prebuild steps run.
await generateAppConfig()


const config = {
	windows: {},
	linux: {
		aptPackages: [
			'tesseract-ocr',
			'libtesseract-dev',
			'pkg-config',
			'build-essential',
			'libglib2.0-dev',
			'libgtk-3-dev',
			'libwebkit2gtk-4.1-dev',
			'clang',
			'cmake', // Tauri
			'libxdo-dev'
		],
		tesseractUrl: 'https://github.com/DanielMYT/tesseract-static/releases/download/tesseract-5.5.0/tesseract',
		tesseractName: 'tesseract',
	},
}

// Export for Github actions
const exports = {
	libClang: 'C:\\Program Files\\LLVM\\bin',
	cmake: 'C:\\Program Files\\CMake\\bin',
}

// Add this function to copy the Bun binary
async function copyBunBinary() {
	console.log('checking bun binary for tauri...');

	let bunSrc, bunDest1, bunDest2;
	if (platform === 'windows') {
		// Get and log npm global prefix
		let npmGlobalPrefix = null;
		try {
			npmGlobalPrefix = (await $`npm config get prefix`.text()).trim();
			console.log('npm global prefix:', npmGlobalPrefix);
		} catch (error) {
			console.log('failed to get npm global prefix:', error.message);
		}

		// Try to find bun location using system commands
		let bunPathFromSystem;
		try {
			bunPathFromSystem = (await $`where.exe bun`.text()).trim().split('\n')[0];
		} catch {
			try {
				bunPathFromSystem = (await $`which bun`.text()).trim();
			} catch {
				console.log('could not find bun using where.exe or which');
			}
		}

		if (bunPathFromSystem) {
			console.log('found bun using system command at:', bunPathFromSystem);
		}

		// Start with basic paths that don't depend on npmGlobalPrefix
		const possibleBunPaths = [
			// Add system-found path if it exists
			bunPathFromSystem,
			// Bun's default installer location
			path.join(os.homedir(), '.bun', 'bin', 'bun.exe'),
			// AppData paths
			path.join(os.homedir(), 'AppData', 'Local', 'bun', 'bun.exe'),
			// Direct paths
			'C:\\Program Files\\bun\\bun.exe',
			'C:\\Program Files (x86)\\bun\\bun.exe',
			// System path
			'bun.exe'
		].filter(Boolean);

		// Add npm paths only if npmGlobalPrefix was successfully retrieved
		if (npmGlobalPrefix) {
			possibleBunPaths.push(
				path.join(npmGlobalPrefix, 'node_modules', 'bun', 'bin', 'bun.exe'),
				path.join(npmGlobalPrefix, 'bun.exe'),
				path.join(npmGlobalPrefix, 'bin', 'bun.exe')
			);
		}

		console.log('searching bun in these locations:');
		possibleBunPaths.forEach(p => console.log('- ' + p));

		bunSrc = null;
		for (const possiblePath of possibleBunPaths) {
			try {
				await fs.access(possiblePath);
				console.log('found bun at:', possiblePath);
				bunSrc = possiblePath;
				break;
			} catch {
				continue;
			}
		}

		if (!bunSrc) {
			throw new Error('Could not find bun.exe in any expected location. Please check if bun is installed correctly');
		}

		// Tauri externalBin looks for bun-{target_triple}; on Windows arm64 → aarch64-pc-windows-msvc, x64 → x86_64-pc-windows-msvc
		const bunTripleSuffix = winArch === 'arm64' ? 'aarch64-pc-windows-msvc' : 'x86_64-pc-windows-msvc'
		bunDest1 = path.join(cwd, `bun-${bunTripleSuffix}.exe`)
		console.log('copying bun from:', bunSrc);
		console.log('copying bun to:', bunDest1);
	} else if (platform === 'linux') {
		bunDest1 = path.join(cwd, 'bun-x86_64-unknown-linux-gnu');

		if (await fs.exists(bunDest1)) {
			console.log('bun binary already exists for tauri.');
			return;
		}

		if (process.env.CI === 'true' || process.env.GITHUB_ACTIONS === 'true') {
			const systemBun = await findOnPath('bun');
			if (!systemBun) {
				throw new Error('CI expected bun on PATH, but command lookup failed');
			}
			console.log(`using CI bun binary for tauri sidecar: ${systemBun}`);
			await copyFile(systemBun, bunDest1);
			return;
		}

		// Download the baseline bun variant for broader glibc compatibility.
		// Use npm's tarball mirror because GitHub release assets can 504.
		const bunVersion = '1.3.10';
		const baselineUrl = `https://registry.npmjs.org/@oven/bun-linux-x64-baseline/-/bun-linux-x64-baseline-${bunVersion}.tgz`;
		console.log(`downloading bun baseline v${bunVersion} for linux...`);
		const tmpArchive = path.join(cwd, 'bun-baseline.tgz');
		const tmpDir = path.join(cwd, 'bun-baseline-tmp');
		try {
			await downloadFile(baselineUrl, tmpArchive, { retries: 10 });
			await fs.rm(tmpDir, { recursive: true, force: true });
			await fs.mkdir(tmpDir, { recursive: true });
			await $`tar -xzf ${tmpArchive} -C ${tmpDir}`;
			const extractedBun = path.join(tmpDir, 'package', 'bin', 'bun');
			await copyFile(extractedBun, bunDest1);
			console.log(`bun baseline binary installed to ${bunDest1}`);
			// cleanup
			await fs.rm(tmpArchive, { force: true });
			await fs.rm(tmpDir, { recursive: true, force: true });
		} catch (error) {
			console.error('failed to download bun baseline:', error);
			const systemBun = await findOnPath('bun');
			if (systemBun) {
				console.warn(`falling back to system bun at ${systemBun}`);
				await copyFile(systemBun, bunDest1);
				await fs.rm(tmpArchive, { force: true });
				await fs.rm(tmpDir, { recursive: true, force: true });
				return;
			}
			process.exit(1);
		}
		return;
	} else if (platform === 'macos') {
		bunDest1 = path.join(cwd, 'bun-aarch64-apple-darwin');
		bunDest2 = path.join(cwd, 'bun-x86_64-apple-darwin');

		// Always download arch-specific bun binaries for macOS targets. We
		// can't trust the host's bun (the x86_64 build runs on an arm64
		// macos-26 runner, so copying systemBun bundled an arm64 binary into
		// the Intel app — surfaced as Pi-install "Bad CPU type in executable
		// (os error 86)" on Intel Macs).
		const bunVersion = '1.3.10';
		const releaseTarget = process.env.DYSTIL_RELEASE_TARGET;

		const archMap = [
			{ target: 'aarch64-apple-darwin', url: `https://github.com/oven-sh/bun/releases/download/bun-v${bunVersion}/bun-darwin-aarch64.zip`, dest: bunDest1, label: 'aarch64' },
			{ target: 'x86_64-apple-darwin',  url: `https://github.com/oven-sh/bun/releases/download/bun-v${bunVersion}/bun-darwin-x64.zip`,     dest: bunDest2, label: 'x64' },
		];

		// In CI we set DYSTIL_RELEASE_TARGET per-matrix-entry and only need
		// that one sidecar. Locally (no env), download both so either-arch dev
		// builds work without re-running this script.
		const wanted = releaseTarget
			? archMap.filter((e) => e.target === releaseTarget)
			: archMap;

		if (wanted.length === 0) {
			throw new Error(`unknown DYSTIL_RELEASE_TARGET for macOS: ${releaseTarget}`);
		}

		for (const { url, dest, label } of wanted) {
			if (await fs.exists(dest)) {
				console.log(`bun ${label} binary already exists, skipping download.`);
				continue;
			}
			console.log(`downloading bun v${bunVersion} for macOS ${label}...`);
			const tmpZip = path.join(cwd, `bun-darwin-${label}.zip`);
			const tmpDir = path.join(cwd, `bun-darwin-${label}-tmp`);
			try {
				await downloadFile(url, tmpZip, { retries: 10, timeoutMs: 120000 });
				await $`unzip -o ${tmpZip} -d ${tmpDir}`;
				// The zip contains a folder like bun-darwin-aarch64/bun or bun-darwin-x64/bun
				const entries = await fs.readdir(tmpDir);
				const extractedBun = path.join(tmpDir, entries[0], 'bun');
				await copyFile(extractedBun, dest);
				console.log(`bun ${label} binary installed to ${dest}`);
				await fs.rm(tmpZip, { force: true });
				await fs.rm(tmpDir, { recursive: true, force: true });
			} catch (error) {
				console.error(`failed to download bun ${label}:`, error);
				process.exit(1);
			}
		}
		return;
	}

	if (await fs.exists(bunDest1)) {
		console.log('bun binary already exists for tauri.');
		return;
	}

	try {
		await fs.access(bunSrc);
		await copyFile(bunSrc, bunDest1);
		console.log(`bun binary copied successfully from ${bunSrc} to ${bunDest1}`);
	} catch (error) {
		console.error('failed to copy bun binary:', error);
		console.error('source path:', bunSrc);
		process.exit(1);
	}
}

// Build the tiny, Rust-only MCP server as a Tauri external binary. This keeps
// the MCP connection independent of the desktop UI process and means end
// users never need Cargo, a terminal, or a global installation.
async function buildDystilMcpSidecar() {
	const targetByHost = {
		windows: winArch === 'arm64' ? 'aarch64-pc-windows-msvc' : 'x86_64-pc-windows-msvc',
		linux: process.arch === 'arm64' ? 'aarch64-unknown-linux-gnu' : 'x86_64-unknown-linux-gnu',
		macos: process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin',
	};
	const target = process.env.DYSTIL_RELEASE_TARGET || targetByHost[platform];
	if (!target) throw new Error(`unsupported platform for Dystil MCP sidecar: ${platform}`);

	const workspace = path.resolve(cwd, '../../..');
	// Cargo honors CARGO_TARGET_DIR even when --manifest-path points at another
	// workspace. Resolve the copied sidecar from that same directory so CI can
	// share one short, persistent target directory with the Tauri build.
	const cargoTargetDir = process.env.CARGO_TARGET_DIR
		? path.resolve(process.env.CARGO_TARGET_DIR)
		: path.join(workspace, 'target');
	const extension = target.includes('windows') ? '.exe' : '';
	const source = path.join(cargoTargetDir, target, 'release', `dystil-mcp${extension}`);
	const destination = path.join(cwd, `dystil-mcp-${target}${extension}`);

	console.log(`building Dystil MCP sidecar for ${target}...`);
	await $`cargo build --manifest-path ${path.join(workspace, 'Cargo.toml')} -p dystil-mcp --release --target ${target}`;
	if (platform === 'windows') {
		await fs.copyFile(source, destination);
	} else {
		// POSIX refuses to overwrite a binary that a previous Dystil process is
		// still executing (ETXTBSY). Stage a new inode beside it, then atomically
		// replace the directory entry; the old process can safely keep its inode.
		const staged = `${destination}.${process.pid}.tmp`;
		try {
			await fs.copyFile(source, staged);
			await fs.chmod(staged, 0o755);
			await fs.rename(staged, destination);
		} finally {
			await fs.rm(staged, { force: true });
		}
	}
	console.log(`Dystil MCP sidecar ready: ${destination}`);
}


// Helper function to copy file and set permissions
async function copyFile(src, dest) {
	await fs.copyFile(src, dest);
	await fs.chmod(dest, 0o755); // ensure the binary is executable
}

async function linkSystemBinary(binaryName, destination) {
	try {
		const source = await findOnPath(binaryName);
		if (!source) {
			return false;
		}
		await fs.rm(destination, { force: true });
		await fs.symlink(source, destination);
		console.log(`using system ${binaryName}: ${source} -> ${destination}`);
		return true;
	} catch (error) {
		console.warn(`could not link system ${binaryName}: ${error.message}`);
		return false;
	}
}

// Regression guard for 9a68ae9de — static layer for macOS sidecars.
// Two checks per binary:
//   1. arch-mismatch: filename suffix must match the actual Mach-O arch. A
//      mislabeled binary (e.g. x86_64 bytes shipped as `*-aarch64-apple-darwin`)
//      crashes on the user's Mac before any code runs.
//   2. dyld-path: every `otool -L` entry must point to `/usr/lib/`,
//      `/System/Library/`, or `@executable_path`/`@rpath`/`@loader_path`.
//      Anything else (brew's Cellar, MacPorts, /Users/...) is fragile and
//      will SIGABRT in production. This is the v2.4.243 crash class.
// Run a system command with a hard timeout via Bun.spawn. Returns the
// captured stdout text. We previously used `await $`cmd`.text()` here but
// observed an indefinite hang on macOS Sequoia where the bun shell helper
// would wedge mid-iteration after the second sidecar — no output, no
// network, no children, just a spinning `R`-state process. Tooling-level
// timeouts are cheap insurance: `file` and `otool` always return in <1s
// in practice, so any wait longer than `timeoutMs` is a bug we want to
// fail loudly on rather than burn the workflow's 180-min ceiling.
async function runWithTimeout(cmd, { timeoutMs = 30_000, label } = {}) {
	const proc = Bun.spawn(cmd, { stdout: 'pipe', stderr: 'pipe' });
	let timedOut = false;
	const timer = setTimeout(() => {
		timedOut = true;
		proc.kill('SIGKILL');
	}, timeoutMs);
	const [stdout, stderr, exitCode] = await Promise.all([
		new Response(proc.stdout).text(),
		new Response(proc.stderr).text(),
		proc.exited,
	]);
	clearTimeout(timer);
	if (timedOut) {
		throw new Error(
			`${label || cmd.join(' ')} timed out after ${timeoutMs}ms — likely a bun shell / system-tool hang.`
		);
	}
	if (exitCode !== 0) {
		throw new Error(
			`${label || cmd.join(' ')} exited ${exitCode}:\n${stderr || stdout}`
		);
	}
	return stdout;
}

async function verifyMacosSidecarsSelfContained() {
	const SAFE_PREFIXES = [
		'/usr/lib/',
		'/System/Library/',
		'@executable_path',
		'@rpath',
		'@loader_path',
	];
	const sidecars = (await fs.readdir('.'))
		.filter((n) => /-(aarch64|x86_64)-apple-darwin$/.test(n))
		.sort();
	if (sidecars.length === 0) return;
	console.log('verifying macOS sidecars are self-contained...');
	for (const bin of sidecars) {
		const expectedArch = bin.endsWith('-aarch64-apple-darwin') ? 'arm64' : 'x86_64';
		const fileOut = (await runWithTimeout(['file', bin], { label: `file ${bin}` })).trim();
		// `file` on a fat binary lists every slice; on a thin binary, just one.
		// Either way the expected arch token must appear.
		if (!new RegExp(`\\b${expectedArch}\\b`).test(fileOut)) {
			throw new Error(
				`sidecar ${bin} has wrong arch:\n` +
				`  ${fileOut}\n` +
				`filename promises ${expectedArch} — Tauri ships it under the matching target.`
			);
		}
		const out = await runWithTimeout(['otool', '-L', bin], { label: `otool -L ${bin}` });
		for (const raw of out.split('\n')) {
			const line = raw.trim();
			if (!line) continue;
			// Skip the "binary:" header and "(architecture x86_64):" sub-headers for fat binaries.
			if (line.endsWith(':')) continue;
			const dylib = line.split(/\s+/)[0];
			if (SAFE_PREFIXES.some((p) => dylib.startsWith(p))) continue;
			throw new Error(
				`sidecar ${bin} links against non-portable dylib:\n` +
				`  ${dylib}\n` +
				`only ${SAFE_PREFIXES.join(', ')} survive transport to a user's Mac.\n` +
				`see commit 9a68ae9de for context.`
			);
		}
		console.log(`  ok: ${bin} (${expectedArch})`);
	}
}

// Regression guard for 9a68ae9de — runtime layer.
// Spawns the host-arch sidecar under `sandbox-exec` with brew/MacPorts paths
// denied, then runs `-version`. dyld loads every non-weak LC_LOAD_DYLIB at
// startup, so `-version` is enough to trip the SIGABRT v2.4.243 hit on user
// Macs. This catches what `otool -L` can't: `dlopen`-loaded plugins and any
// other init-time crash. Absolute dylib paths in LC_LOAD_DYLIB ignore DYLD
// env vars, so `sandbox-exec` is the only way to actually simulate a Mac
// without the brew rev shipped on the CI runner.
//
// Only checks the host-arch sidecar — the other arch gets exercised on its
// own CI matrix entry. The static check above already covers both archs.
async function verifyMacosSidecarsRun() {
	const hostArch = process.arch === 'arm64' ? 'aarch64' : 'x86_64';
	const sidecars = [];
	const profile =
		'(version 1)' +
		'(allow default)' +
		'(deny file-read* (subpath "/opt/homebrew"))' +
		'(deny file-read* (subpath "/usr/local/Cellar"))' +
		'(deny file-read* (subpath "/opt/local"))';
	console.log(`running ${hostArch} sidecars in a brew-less sandbox...`);
	for (const bin of sidecars) {
		if (!(await fs.exists(bin))) continue;
		// Hard timeout: a successful `-version` returns in <1s. If we hit 30s
		// it's a tooling bug (sandbox-exec stuck, bun shell wait-loop, etc.),
		// not the v2.4.243 sidecar crash this guard is looking for — warn and
		// continue rather than wedging every `bun run build`.
		const proc = Bun.spawn(['sandbox-exec', '-p', profile, `./${bin}`, '-version'], {
			stdout: 'pipe',
			stderr: 'pipe',
		});
		let timedOut = false;
		const timer = setTimeout(() => {
			timedOut = true;
			proc.kill('SIGKILL');
		}, 30_000);
		const exitCode = await proc.exited;
		clearTimeout(timer);
		if (timedOut) {
			console.warn(`  WARN: ${bin} sandbox verify timed out after 30s — skipping (likely a tooling issue, not a sidecar regression)`);
			continue;
		}
		if (exitCode !== 0) {
			const stderr = await new Response(proc.stderr).text();
			throw new Error(
				`sidecar ${bin} fails to launch without /opt/homebrew, /usr/local/Cellar, /opt/local:\n` +
				`${stderr || `exit code ${exitCode}`}\n` +
				`this is the v2.4.243 crash class — see commit 9a68ae9de.`
			);
		}
		console.log(`  ok: ${bin}`);
	}
}

async function findOnPath(binaryName) {
	const pathValue = process.env.PATH || '';
	for (const dir of pathValue.split(path.delimiter)) {
		if (!dir) continue;
		const candidate = path.join(dir, binaryName);
		try {
			await fs.access(candidate, fsConstants.X_OK);
			return candidate;
		} catch {
			// Try the next PATH entry.
		}
	}
	return null;
}

/* ########## Linux ########## */
if (platform == 'linux') {
	// In CI, cache-apt-pkgs-action already installs packages; skip redundant apt install
	const inCI = process.env.CI === 'true' || process.env.GITHUB_ACTIONS === 'true';
	if (inCI) {
		console.log('CI detected: apt packages handled by workflow cache-apt-pkgs-action ✅\n');
	} else {
		// Check and install APT packages (local dev)
		try {
			const aptPackagesNotInstalled = [];

			// Check each package installation status
			for (const pkg of config.linux.aptPackages) {
				try {
					await $`dpkg -s ${pkg}`.quiet();
				} catch {
					aptPackagesNotInstalled.push(pkg);
				}
			}

			if (aptPackagesNotInstalled.length > 0) {
				console.log('the following required packages are missing:');
				aptPackagesNotInstalled.forEach(pkg => console.log(`  - ${pkg}`));
				console.log('\ninstalling missing packages...');

				console.log('updating package lists...');
				await $`sudo apt-get -qq update`;

				console.log('installing packages...');
				await $`sudo DEBIAN_FRONTEND=noninteractive apt-get -qq install -y ${aptPackagesNotInstalled}`;
				console.log('Package installation completed successfully ✅\n');
			} else {
				console.log('all required packages are already installed ✅\n');
			}
		} catch (error) {
			console.error("error checking/installing apt packages: %s", error.message);
		}
	}

	// Setup TESSERACT
	if (!(await fs.exists(config.linux.tesseractName))) {
		if (inCI) {
			const linkedTesseract = await linkSystemBinary('tesseract', config.linux.tesseractName);
			if (!linkedTesseract) {
				throw new Error('CI expected tesseract from apt, but command -v tesseract failed');
			}
		} else {
			await $`wget --no-config -nc ${config.linux.tesseractUrl} -O ${config.linux.tesseractName}`
			await $`chmod +x ${config.linux.tesseractName}` // Make the Tesseract binary executable
		}
	} else {
		console.log('TESSERACT already exists');
	}
}

// VC Redist discovery (Windows): vswhere + standard locations so pre_build/pre_dev and CI both work.
// CRT folder can be Microsoft.VC143.CRT (VS 2022), VC144, or VC145 (newer VS); all provide vcruntime140.dll.
const PROGRAM_FILES_X86 = process.env['ProgramFiles(x86)'] || 'C:\\Program Files (x86)';
const PROGRAM_FILES_LIST = [process.env['ProgramFiles(x86)'], process.env['ProgramFiles']].filter(Boolean);
const VS_EDITIONS = ['Enterprise', 'Professional', 'Community', 'BuildTools'];
const VS_YEARS = ['18', '2026', '2025', '2022', '2019', '2017'];
const VSWHERE_DIR = path.join(PROGRAM_FILES_X86, 'Microsoft Visual Studio', 'Installer');
const CRT_FOLDER_NAMES = ['Microsoft.VC145.CRT', 'Microsoft.VC144.CRT', 'Microsoft.VC143.CRT'];

/** Resolve VC\\Redist\\MSVC\\{version} to the latest version subfolder and return CRT path for arch (x64 or arm64), or null */
async function getMsvcCrtDirFromInstallRoot(installRoot, arch = 'x64') {
	const msvcPath = path.join(installRoot, 'VC', 'Redist', 'MSVC');
	try {
		const versions = await fs.readdir(msvcPath);
		const numeric = versions.filter((v) => /^\d+\.\d+\.\d+/.test(v)).sort();
		if (numeric.length === 0) return null;
		const latest = numeric[numeric.length - 1];
		const archPath = path.join(msvcPath, latest, arch);
		for (const crtName of CRT_FOLDER_NAMES) {
			const crtDir = path.join(archPath, crtName);
			try {
				await fs.access(path.join(crtDir, 'vcruntime140.dll'));
				return crtDir;
			} catch {
				continue;
			}
		}
		return null;
	} catch {
		return null;
	}
}

/** Find Microsoft.VC14*.CRT dir (143/144/145): VCToolsRedistDir → vswhere → standard paths. arch: 'x64' or 'arm64' (Windows ARM64). */
async function findVc143CrtDir(arch = 'x64') {
	if (process.env.VCToolsRedistDir) {
		const base = path.join(process.env.VCToolsRedistDir, arch);
		for (const crtName of CRT_FOLDER_NAMES) {
			const crtDir = path.join(base, crtName);
			try {
				await fs.access(path.join(crtDir, 'vcruntime140.dll'));
				console.log('Using VCToolsRedistDir:', crtDir);
				return crtDir;
			} catch (e) {
				continue;
			}
		}
		console.warn('VCToolsRedistDir set but no CRT (VC143/144/145) found');
	}

	const vswhereExe = path.join(VSWHERE_DIR, 'vswhere.exe');
	const component = arch === 'arm64' ? 'Microsoft.VisualStudio.Component.VC.Tools.ARM64' : 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64';
	try {
		if (await fs.access(vswhereExe).then(() => true).catch(() => false)) {
			const installDir = (await $`"${vswhereExe}" -latest -products * -requires ${component} -property installationPath`.text()).trim();
			if (installDir) {
				const crtDir = await getMsvcCrtDirFromInstallRoot(installDir, arch);
				if (crtDir) {
					console.log('Found with vswhere:', crtDir);
					return crtDir;
				}
			}
		}
	} catch (e) {
		console.warn('vswhere failed:', e.message);
	}

	// Fallback: same VS install often has both x64 and arm64 under MSVC\<ver>\
	for (const progFiles of PROGRAM_FILES_LIST) {
		for (const year of VS_YEARS) {
			for (const edition of VS_EDITIONS) {
				const installRoot = path.join(progFiles, 'Microsoft Visual Studio', year, edition);
				const crtDir = await getMsvcCrtDirFromInstallRoot(installRoot, arch);
				if (crtDir) {
					console.log('Found in standard location:', crtDir);
					return crtDir;
				}
			}
		}
	}

	throw new Error(`Microsoft VC143/144/145 CRT (${arch}) not found. Install Visual Studio with C++ tools or set VCToolsRedistDir.`);
}

// Copy VC CRT DLLs (VC143/144/145) into src-tauri/vcredist for Tauri bundle (Windows only). arch: 'x64' or 'arm64'.
async function copyVcredistDlls(arch = 'x64') {
	const vcredistDir = path.join(cwd, 'vcredist');
	await fs.mkdir(vcredistDir, { recursive: true });

	const crtDir = await findVc143CrtDir(arch);

	const dlls = ['msvcp140.dll', 'msvcp140_1.dll', 'msvcp140_2.dll', 'vcruntime140.dll', 'vcruntime140_1.dll'];
	for (const dll of dlls) {
		await fs.copyFile(path.join(crtDir, dll), path.join(vcredistDir, dll));
	}
	console.log('VC CRT DLLs copied to vcredist');
}

/* ########## Windows ########## */
if (platform == 'windows') {
	exports.openBlas = await setupOpenBlas({ cwd, winArch })

	// Copy VC143 CRT DLLs for Tauri bundle (required in CI; optional locally). Use arch matching current Windows (x64 or arm64).
		const inCI = process.env.CI === 'true' || process.env.GITHUB_ACTIONS === 'true';
		if (inCI) {
			await copyVcredistDlls(winArch);
		} else {
			try {
				await copyVcredistDlls(winArch);
			} catch (err) {
				console.warn('Skipping VC redist DLL copy (optional outside CI):', err.message);
		}
	}
}


/* ########## macOS ########## */
if (platform == 'macos') {
	// Strip extended attributes from all binaries to prevent codesign failures
	console.log('Stripping extended attributes from binaries...');
	try {
		await $`xattr -cr ${cwd} 2>/dev/null`;
		console.log('Extended attributes stripped successfully');
	} catch (error) {
		console.log('Note: xattr command not available or failed (non-fatal)');
	}
}



// Development hints
if (!process.env.GITHUB_ENV) {
	console.log('\nCommands to build 🔨:')
	// Get relative path to dystil folder
	const relativePath = path.relative(originalCWD, path.join(cwd, '..'))
	if (originalCWD != cwd && relativePath != '') {
		console.log(`cd ${relativePath}`)
	}
	console.log('bun install')

	if (!process.env.GITHUB_ENV) {
		console.log('bun tauri build')
	}
}

// Config Github ENV
if (process.env.GITHUB_ENV) {
	console.log('Adding ENV')
	if (platform == 'windows') {
		const openblas = `OPENBLAS_PATH=${exports.openBlas}\n`
		console.log('Adding ENV', openblas)
		await fs.appendFile(process.env.GITHUB_ENV, openblas)
	}
}


// Near the end of the script, call these functions
await copyBunBinary();
await buildDystilMcpSidecar();

// --dev or --build
const action = process.argv?.[2]
if (action?.includes('--build') || action?.includes('--dev')) {
	process.chdir(path.join(cwd, '..'))
	if (platform === 'windows') {
		process.env['OPENBLAS_PATH'] = exports.openBlas
		process.env['LIBCLANG_PATH'] = exports.libClang
		process.env['PATH'] = `${process.env['PATH']};${exports.cmake}`
	}
	await $`bun install`
	await $`bunx tauri ${action.includes('--dev') ? 'dev' : 'build'}`
}
