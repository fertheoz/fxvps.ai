import type { Metadata } from "next";
import "./globals.css";
import { Providers } from "@/components/shell/providers";
import { AppShell } from "@/components/shell/app-shell";

export const metadata: Metadata = {
  title: "fxvps.ai Back Office",
  description: "Broker back office: clients, groups, symbols, risk, LP connections, reports.",
};

const themeScript = `try{var p=JSON.parse(localStorage.getItem("fxvps-bo-prefs")||"{}");if((p.theme||"dark")==="dark")document.documentElement.classList.add("dark");if(p.locale)document.documentElement.lang=p.locale}catch(e){document.documentElement.classList.add("dark")}`;

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: themeScript }} />
      </head>
      <body className="antialiased">
        <Providers>
          <AppShell>{children}</AppShell>
        </Providers>
      </body>
    </html>
  );
}
