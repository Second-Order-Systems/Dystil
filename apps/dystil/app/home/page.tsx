"use client";

import { useSearchParams } from "next/navigation";
import { HomeRoute } from "@/components/dystil/home/home-route";

export default function HomePage() {
  const searchParams = useSearchParams();
  return <HomeRoute initialText={searchParams.get("initial") ?? ""} />;
}
