"use client";

/**
 * Shares one HomeSource across the shell and the routes beneath it.
 *
 * The shell needs the queue count and the running job; the pages need the
 * items and the actions. Both come from the same source, and the provider
 * lives in app/home/layout.tsx so navigating between routes does not reset it.
 */

import { createContext, useContext } from "react";
import { useHomeSource } from "./index";
import type { HomeSource } from "./types";

const HomeContext = createContext<HomeSource | null>(null);

export function HomeProvider({ children }: { children: React.ReactNode }) {
  const source = useHomeSource();
  return <HomeContext.Provider value={source}>{children}</HomeContext.Provider>;
}

export function useHome(): HomeSource {
  const source = useContext(HomeContext);
  if (!source) throw new Error("useHome must be used within a HomeProvider");
  return source;
}
