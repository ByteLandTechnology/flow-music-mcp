// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

// Computes name-based UUID version 5 values, as the page does for lyrics identifiers.

/** Returns the UUID version 5 of `name` in namespace `namespace`. */
export async function uuidV5(name: string, namespace: string): Promise<string> {
  const hex = namespace.replace(/-/g, "");
  const ns = new Uint8Array(16);
  for (let i = 0; i < 16; i += 1) {
    ns[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  const text = new TextEncoder().encode(name);
  const data = new Uint8Array(ns.length + text.length);
  data.set(ns, 0);
  data.set(text, ns.length);
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-1", data)).slice(0, 16);
  digest[6] = (digest[6] & 0x0f) | 0x50;
  digest[8] = (digest[8] & 0x3f) | 0x80;
  const out = Array.from(digest, (b) => b.toString(16).padStart(2, "0")).join("");
  return `${out.slice(0, 8)}-${out.slice(8, 12)}-${out.slice(12, 16)}-${out.slice(16, 20)}-${out.slice(20)}`;
}
