// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

// Browser runtime bundle entry point that registers window.__flowMusicMcp once per document.

import { api, fetchBinary, readStream, upload } from "./api";
import { checkAuth } from "./check_auth";
import { uuidV5 } from "./uuid";

/** Methods that the Rust engine calls through CDP `Runtime.evaluate`. */
interface MusicRuntime {
  /** Reports the sign-in state of the page. */
  checkAuth: typeof checkAuth;
  /** Sends one internal API request. */
  api: typeof api;
  /** Reads a job stream for a bounded time. */
  readStream: typeof readStream;
  /** Uploads one file as multipart form data. */
  upload: typeof upload;
  /** Downloads one API resource as Base64. */
  fetchBinary: typeof fetchBinary;
  /** Computes a UUID version 5 value. */
  uuidV5: typeof uuidV5;
}

declare global {
  interface Window {
    /** Runtime object that the Rust engine installs once per document. */
    __flowMusicMcp?: MusicRuntime;
  }
}

if (!window.__flowMusicMcp) {
  window.__flowMusicMcp = { checkAuth, api, readStream, upload, fetchBinary, uuidV5 };
}
