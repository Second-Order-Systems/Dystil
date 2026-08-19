import "@testing-library/jest-dom/vitest";
import { JSDOM } from "jsdom";

if (typeof window === "undefined") {
  const dom = new JSDOM("<!doctype html><html><body></body></html>", {
    url: "http://localhost",
  });

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const globalWithDom = globalThis as any;

  globalWithDom.window = dom.window;
  globalWithDom.document = dom.window.document;
  globalWithDom.navigator = dom.window.navigator;
  globalWithDom.location = dom.window.location;
  globalWithDom.HTMLElement = dom.window.HTMLElement;
}

// Bun's Node global `localStorage` accessor is undefined unless Bun is started
// with a local-storage file. Supply an in-memory browser-compatible store for
// the jsdom test environment instead of making the suite depend on that flag.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const globalWithDom = globalThis as any;
if (globalWithDom.localStorage === undefined) {
  const values = new Map<string, string>();
  const storage: Storage = {
    get length() {
      return values.size;
    },
    clear() {
      values.clear();
    },
    getItem(key) {
      return values.get(key) ?? null;
    },
    key(index) {
      return [...values.keys()][index] ?? null;
    },
    removeItem(key) {
      values.delete(key);
    },
    setItem(key, value) {
      values.set(String(key), String(value));
    },
  };
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: storage,
  });
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: storage,
  });
}
