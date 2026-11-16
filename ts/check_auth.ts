// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

// Reports whether the page has a usable Google Flow Music session.

import { api } from "./api";
import { isExpired, readSession } from "./session";

/** Sign-in state reported to the Rust side. */
export interface AuthState {
  /** True when the session cookie exists and `GET /users/me` succeeds. */
  signed_in: boolean;
  /** True when the page has a session cookie. */
  has_session: boolean;
  /** True when the access token expired and the page must refresh it. */
  token_expired: boolean;
  /** Supabase user identifier, if known. */
  user_id: string | null;
  /** Email address of the signed-in account, if known. */
  email: string | null;
  /** HTTP status of `GET /users/me`, or 0 when the runtime did not send it. */
  status: number;
}

/** Checks the session cookie and confirms it with `GET /users/me`. */
export async function checkAuth(): Promise<AuthState> {
  const session = readSession();
  if (!session) {
    return { signed_in: false, has_session: false, token_expired: false, user_id: null, email: null, status: 0 };
  }
  if (isExpired(session)) {
    return { signed_in: false, has_session: true, token_expired: true, user_id: session.userId, email: session.email, status: 0 };
  }
  const me = await api("GET", "/users/me");
  return {
    signed_in: me.status === 200,
    has_session: true,
    token_expired: me.status === 401,
    user_id: session.userId,
    email: session.email,
    status: me.status,
  };
}
