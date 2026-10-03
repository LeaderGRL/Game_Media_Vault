import type { ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

interface ExternalLinkProps {
  href: string;
  children: ReactNode;
}

/**
 * A link that opens in the system browser rather than in the app's own window. The app's
 * capability lets it open only the sites it names (src-tauri/capabilities/default.json).
 */
export function ExternalLink({ href, children }: ExternalLinkProps) {
  return (
    <a
      href={href}
      onClick={(event) => {
        event.preventDefault();
        void openUrl(href);
      }}
    >
      {children}
    </a>
  );
}
