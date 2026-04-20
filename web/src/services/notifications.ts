// Dormant SSE-notifications wiring for v0.1. The MCP SDK already opens
// a GET stream on the transport; we just tap into
// `fallbackNotificationHandler` and log the frames that come through.
// No UI surface yet; this is instrumentation.

import type { Notification } from "@modelcontextprotocol/sdk/types.js";

import type { McpSession } from "../mcp";

export interface AlertLogger {
  install: () => void;
  uninstall: () => void;
}

const ALERT_METHOD = "notifications/alert_triggered";

/**
 * Attach a log-only notification handler to the MCP session. Returns an
 * object with `install`/`uninstall` so components can wire it into a
 * Solid lifecycle.
 */
export function createAlertLogger(session: McpSession): AlertLogger {
  let installed = false;

  return {
    install() {
      if (installed) return;
      installed = true;
      session.onNotification((n: Notification) => {
        if (n.method === ALERT_METHOD) {
          // eslint-disable-next-line no-console
          console.log("[alert]", n.params);
        } else {
          // eslint-disable-next-line no-console
          console.log("[mcp-notification]", n.method, n.params);
        }
      });
    },
    uninstall() {
      if (!installed) return;
      installed = false;
      // Reset to a no-op so the session stops logging after unmount.
      session.onNotification(() => {
        /* no-op */
      });
    },
  };
}
