"use client";

import { usePathname, useRouter } from "next/navigation";

const routeFor = (path: string) => path === "worth-fixing" ? "/home" : `/home/${path}`;

export function Sidebar() {
  const pathname = usePathname();
  const router = useRouter();
  const navigate = (path: string) => router.push(routeFor(path));
  const primary = pathname.endsWith("/ready") ? "ready" : pathname.endsWith("/ask") ? "ask" : "worth-fixing";
  return <aside className="flex min-h-0 flex-col border-r border-[#dfded9] bg-white py-[27px]">
    <button type="button" className="mb-[28px] w-fit px-[26px] text-left text-[18px] font-semibold tracking-[0.28em] text-[#07110e] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-[#078260]" onClick={() => navigate("worth-fixing")} aria-label="Dystil home">D<span className="text-[#087d63]">Y</span>STIL</button>
    <nav aria-label="Primary navigation">
      <SidebarButton active={primary === "worth-fixing"} onClick={() => navigate("worth-fixing")}>Worth fixing</SidebarButton>
      <SidebarButton active={primary === "ready"} onClick={() => navigate("ready")}>Ready to use</SidebarButton>
      <SidebarButton active={primary === "ask"} onClick={() => navigate("ask")}>Ask for a fix</SidebarButton>
    </nav>
    <div className="mx-[26px] mt-auto border-t border-[#e7e5df] pt-[21px]">
      <div className="mb-[29px]"><p className="flex items-center gap-[10px] text-[18px] leading-[1.25] text-[#42464a]"><span className="h-[9px] w-[9px] shrink-0 rounded-full bg-[#12a77a]" aria-hidden="true" />Watching</p><p className="mt-[7px] max-w-[190px] text-[16px] leading-[1.45] text-[#9398a0]">Nothing has left this computer. It cannot.</p></div>
      <nav className="grid gap-[11px]" aria-label="Secondary navigation">
        <FooterLink active={pathname.endsWith("/privacy")} onClick={() => navigate("privacy")}>What stays on this<br />computer</FooterLink>
        <FooterLink active={pathname.includes("/settings") && pathname.includes("invite")} onClick={() => router.push("/home/settings?tab=Invite%20your%20team")}>Invite your team</FooterLink>
        <FooterLink active={pathname.includes("/settings")} onClick={() => navigate("settings")}>Settings</FooterLink>
      </nav>
    </div>
  </aside>;
}

function SidebarButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return <button type="button" onClick={onClick} className={`relative block h-[50px] w-full px-[28px] text-left text-[20px] transition-colors focus-visible:z-10 focus-visible:outline-none focus-visible:shadow-[inset_0_0_0_1px_#078260] ${active ? "bg-[#def3ed] text-[#006a51]" : "text-[#42464a] hover:bg-[#f5f7f5]"}`}>{active && <span className="absolute inset-y-0 left-0 w-[2px] bg-[#0aa275]" aria-hidden="true" />}{children}</button>;
}

function FooterLink({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return <button type="button" onClick={onClick} className={`w-full text-left text-[17px] leading-[1.25] hover:text-[#006a51] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#078260] ${active ? "font-medium text-[#006a51]" : "text-[#42464a]"}`}>{children}</button>;
}
