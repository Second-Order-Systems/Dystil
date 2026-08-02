"use client";

import { useState, type ReactNode } from "react";

export function PageHeading({ title, description }: { title: string; description: ReactNode }) {
  return <header><h1 className="text-balance text-[31px] font-normal leading-[1.22] tracking-[-0.035em] text-black">{title}</h1><p className="mt-[20px] max-w-[780px] text-[20px] leading-[1.75] text-[#4f5660]">{description}</p></header>;
}

export function TextAction({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return <button type="button" onClick={onClick} className="shrink-0 text-[15px] text-[#0f6e56] hover:text-[#094b3b] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#1d9e75]">{children}</button>;
}

export function PrivacyCard({ title, action, accent = false, children }: { title: string; action?: ReactNode; accent?: boolean; children: ReactNode }) {
  return <section className={`mt-[13px] rounded-[12px] border bg-white px-[21px] py-[18px] ${accent ? "border-[#c9e7db]" : "border-[#e7e7e2]"}`}><div className="flex items-start justify-between gap-6"><div className="min-w-0 flex-1"><h2 className={`text-[18px] font-medium ${accent ? "text-[#0f6e56]" : "text-[#1a1c20]"}`}>{title}</h2><div className="mt-[6px] text-[16px] leading-[1.6] text-[#60636b]">{children}</div></div>{action}</div></section>;
}

export function ChipRow({ items, firstActive = false }: { items: string[]; firstActive?: boolean }) {
  return <div className="mt-[14px] flex flex-wrap gap-[8px]">{items.map((item, index) => <span key={item} className={`rounded-full border px-[13px] py-[6px] text-[14px] ${firstActive && index === 0 ? "border-[#c9e7db] bg-[#f3f8f5] text-[#0f6e56]" : "border-[#e1e1dc] text-[#60636b]"}`}>{item}</span>)}</div>;
}

export function DataChips({ label, items }: { label: string; items: string[] }) {
  const [visible, setVisible] = useState(items);
  return <div className="mt-[20px]"><p className="mb-[9px] text-[14px] text-[#92969e]">{label}</p><div className="flex flex-wrap gap-[8px]">{visible.map((item) => <button key={item} type="button" onClick={() => setVisible((current) => current.filter((value) => value !== item))} className="rounded-full bg-[#f3f8f5] px-[13px] py-[6px] text-[14px] text-[#26312d]">{item} <span className="ml-1 text-[#9a9da5]">×</span></button>)}</div></div>;
}
