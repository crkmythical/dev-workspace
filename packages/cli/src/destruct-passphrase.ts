#!/usr/bin/env bun
/**
 * destruct-passphrase — Print the remote self-destruct passphrase.
 *
 * The passphrase is a deterministic function of the vault passphrase (HKDF), so
 * it can be re-derived at any time instead of having to be saved from the
 * one-time banner shown when the key hash was first created.
 *
 * The vault passphrase is taken from $PASSWORD if set, else prompted / piped.
 */
import { loadConfig } from "@sdw/core/workspace";
import { deriveDestructKey, destructKeyToPassphrase } from "@sdw/infra";
import { readPassphrase } from "./lib/passphrase.ts";

const config = loadConfig(process.env);
const passphrase = await readPassphrase(config.password, "Vault passphrase: ");
if (!passphrase) {
  console.error("ERROR: Passphrase cannot be empty.");
  process.exit(1);
}

console.log(destructKeyToPassphrase(deriveDestructKey(passphrase)));

if (!config.features.remoteDestruct) {
  console.error("Note: REMOTE_DESTRUCT_ENABLED is not set, so the remote endpoint is off.");
}
