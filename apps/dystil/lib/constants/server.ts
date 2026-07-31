/**
 * App-local focus/notification server — also serves app icons and the
 * installed-app list. Keep in sync with `DEFAULT_FOCUS_PORT` in
 * `src-tauri/src/server.rs`.
 *
 * The webview can't read `DYSTIL_FOCUS_PORT`; callers that must honour that
 * override should read the port from the `getAppServerConfig` Tauri command.
 */
export const DEFAULT_APP_SERVER_PORT = 11735;

export const APP_SERVER_ORIGIN = `http://localhost:${DEFAULT_APP_SERVER_PORT}`;

/** Build a URL against the app-local server, e.g. `appServerUrl("/installed-apps")`. */
export function appServerUrl(path: string): string {
  return `${APP_SERVER_ORIGIN}${path.startsWith("/") ? path : `/${path}`}`;
}
