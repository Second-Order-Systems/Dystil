import path from 'path'
import { generateAppConfig } from './app-config.mjs'

const repoRoot = path.resolve(import.meta.dirname, '../../..')

generateAppConfig()
	.then(({ sourceFile, outFile }) => {
		console.log(
			`[gen-app-config] wrote ${path.relative(repoRoot, outFile)} from ${path.relative(repoRoot, sourceFile)}`,
		)
	})
	.catch((error) => {
		console.error(error)
		process.exit(1)
	})
