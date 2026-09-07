/**
 * Static by construction.
 *
 * There is no server here: no forms, no accounts, no analytics endpoint. A
 * site about a product that sends nothing anywhere should not itself be
 * collecting anything, and exporting statically makes that a property of the
 * build rather than a promise in a footer.
 */
const config = {
  // The repository is inside a folder that has its own lockfile, which makes
  // Next guess the wrong project root and warn about it on every build.
  outputFileTracingRoot: import.meta.dirname,
  output: "export",
  images: { unoptimized: true },
  reactStrictMode: true,
};

export default config;
