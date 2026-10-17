// SPDX-License-Identifier: MIT OR Apache-2.0
/** JSON numbers outside JS's exact integer range must not silently alter Rust contracts. */
export function checkNumbers(value: unknown): void {
  if (typeof value === "number" && (!Number.isFinite(value) || (Number.isInteger(value) && !Number.isSafeInteger(value)))) {
    throw new Error("Contract contains a number that JavaScript cannot represent exactly; use the raw JSON WASM API");
  }
  if (Array.isArray(value)) value.forEach(checkNumbers);
  else if (value && typeof value === "object") Object.values(value).forEach(checkNumbers);
}
export function parseContract<T>(json: string): T {
  const value: unknown = JSON.parse(json);
  checkNumbers(value);
  return value as T;
}
export function encodeContract(value: unknown): string {
  checkNumbers(value);
  return JSON.stringify(value);
}
/** wasm-bindgen coerces BigInt arguments modulo 2^64; reject wrapping at this boundary. */
export function checkTime(now: bigint): void {
  if (typeof now !== "bigint" || now < 0n || now > 0xffff_ffff_ffff_ffffn) {
    throw new Error("Time must be an unsigned 64-bit BigInt");
  }
}

