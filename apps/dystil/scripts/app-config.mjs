import fs from 'fs'
import { mkdir, readFile, writeFile } from 'fs/promises'
import path from 'path'
import { fileURLToPath } from 'url'

// Shared app-config resolution for every JS-side entrypoint.
//
// We intentionally have multiple callers:
// - next.config.mjs: Next can start/build without Tauri preflight
// - scripts/pre_build.js: normal app workflows come through here
// - src-tauri/build.rs: Rust can compile without the JS pipeline
//
// The source of truth is config/app-config.<channel>.json.
// The resolved artifact is lib/generated/app-config.json.
export const VALID_CHANNELS = ['local', 'beta', 'prod']

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)
const appRoot = path.resolve(__dirname, '..')
const configDir = path.join(appRoot, 'config')
const outFile = path.join(appRoot, 'lib/generated/app-config.json')

export function resolveBuildChannel(env = process.env) {
	const explicit = env.DYSTIL_BUILD_CHANNEL?.trim().toLowerCase()
	if (explicit) {
		if (!VALID_CHANNELS.includes(explicit)) {
			throw new Error(
				`Invalid DYSTIL_BUILD_CHANNEL="${env.DYSTIL_BUILD_CHANNEL}". Expected one of: ${VALID_CHANNELS.join(', ')}`,
			)
		}
		return explicit
	}

	switch (env.npm_lifecycle_event) {
		case 'prebuild':
			return 'prod'
		case 'predev':
			return 'local'
		default:
			return env.NODE_ENV === 'production' ? 'prod' : 'local'
	}
}

export function getSourceConfigPath(channel) {
	return path.join(configDir, `app-config.${channel}.json`)
}

export function getGeneratedConfigPath() {
	return outFile
}

function normalizeConfig(config) {
	if (!config || typeof config !== 'object' || Array.isArray(config)) {
		throw new Error('config must be a JSON object')
	}

	// Cloud configuration deliberately does not live in a checked-in frontend
	// artifact. Packaged cloud builds receive their endpoint from Rust build.rs.
	return {}
}

export async function generateAppConfig(channel = resolveBuildChannel()) {
	const sourceFile = getSourceConfigPath(channel)
	const raw = await readFile(sourceFile, 'utf8')
	const normalized = normalizeConfig(JSON.parse(raw))
	await mkdir(path.dirname(outFile), { recursive: true })
	await writeFile(outFile, `${JSON.stringify(normalized, null, 2)}\n`, 'utf8')
	return { channel, sourceFile, outFile }
}

export function generateAppConfigSync(channel = resolveBuildChannel()) {
	const sourceFile = getSourceConfigPath(channel)
	const raw = fs.readFileSync(sourceFile, 'utf8')
	const normalized = normalizeConfig(JSON.parse(raw))
	fs.mkdirSync(path.dirname(outFile), { recursive: true })
	fs.writeFileSync(outFile, `${JSON.stringify(normalized, null, 2)}\n`, 'utf8')
	return { channel, sourceFile, outFile }
}
