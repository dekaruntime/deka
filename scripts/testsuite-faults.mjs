// A failed `deka add` is not inherently a fixture failure. Keep this narrow:
// only a transport failure, malformed registry response, rate limit, or 5xx
// from the registry/CDN means the runner could not obtain the bytes to test.
// A missing package (404), a bad archive, integrity rejection, or a package
// that installs and then breaks compilation remains a real fixture failure.
export function registryInstallBlockReason(error) {
  const message = String(error ?? "");
  if (/package\s+@?[^\s]+\s+not found in deka\.gg registry/i.test(message)) {
    return null;
  }
  if (
    /failed to contact deka\.gg registry|failed to download tarball|failed to parse deka\.gg registry metadata/i.test(
      message
    )
  ) {
    return "registry fetch failed; fixture did not run";
  }
  const status = message.match(/\bstatus\s+(\d{3})\b/i);
  if (status) {
    const code = Number(status[1]);
    if (code === 429 || code >= 500) {
      return `registry fetch returned HTTP ${code}; fixture did not run`;
    }
  }
  return null;
}
