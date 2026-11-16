// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

// Reads the Supabase session that the Google Flow Music page keeps in cookies.

/** Name prefix of the Supabase session cookie and its numbered chunks. */
const SESSION_COOKIE = "sb-sb-auth-token";

/** Seconds before expiry when the runtime treats the access token as expired. */
const EXPIRY_MARGIN_S = 60;

/** Decoded fields of the Supabase session that the runtime uses. */
export interface SessionInfo {
  /** Bearer access token for internal API requests. */
  accessToken: string;
  /** Expiry time of the access token in Unix seconds. */
  expiresAt: number;
  /** Supabase user identifier. */
  userId: string;
  /** Email address of the signed-in account, if present. */
  email: string | null;
}

/** Decodes a base64url string to UTF-8 text. */
function decodeBase64Url(raw: string): string {
  const b64 = raw.replace(/-/g, "+").replace(/_/g, "/");
  const padded = b64 + "=".repeat((4 - (b64.length % 4)) % 4);
  const bytes = Uint8Array.from(atob(padded), (c) => c.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}

/** Collects the session cookie value from the unchunked cookie or from its numbered chunks. */
function sessionCookieValue(): string | null {
  const pairs = document.cookie
    .split("; ")
    .map((entry) => {
      const at = entry.indexOf("=");
      return at < 0 ? [entry, ""] : [entry.slice(0, at), entry.slice(at + 1)];
    })
    .filter(([name]) => name === SESSION_COOKIE || name.startsWith(`${SESSION_COOKIE}.`));
  if (pairs.length === 0) {
    return null;
  }
  const whole = pairs.find(([name]) => name === SESSION_COOKIE);
  if (whole) {
    return decodeURIComponent(whole[1]);
  }
  const chunks = pairs
    .map(([name, value]) => [Number(name.slice(SESSION_COOKIE.length + 1)), value] as const)
    .filter(([index]) => Number.isInteger(index))
    .sort((a, b) => a[0] - b[0]);
  return chunks.map(([, value]) => decodeURIComponent(value)).join("");
}

/** Returns the decoded session, or `null` when the page has no session cookie. */
export function readSession(): SessionInfo | null {
  const raw = sessionCookieValue();
  if (!raw) {
    return null;
  }
  const body = raw.startsWith("base64-") ? decodeBase64Url(raw.slice(7)) : raw;
  const parsed: unknown = JSON.parse(body);
  if (typeof parsed !== "object" || parsed === null) {
    return null;
  }
  const record = parsed as Record<string, unknown>;
  const user = (record.user ?? {}) as Record<string, unknown>;
  if (typeof record.access_token !== "string") {
    return null;
  }
  return {
    accessToken: record.access_token,
    expiresAt: typeof record.expires_at === "number" ? record.expires_at : 0,
    userId: typeof user.id === "string" ? user.id : "",
    email: typeof user.email === "string" ? user.email : null,
  };
}

/** Returns `true` when the access token expires within the safety margin. */
export function isExpired(session: SessionInfo): boolean {
  return session.expiresAt > 0 && session.expiresAt - EXPIRY_MARGIN_S <= Date.now() / 1000;
}
