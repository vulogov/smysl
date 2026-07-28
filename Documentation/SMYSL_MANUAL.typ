// smysl — The Manual
// A complete, worked guide to every workflow — the CLI, the document format,
// and the library beneath both.
//
// Compile with:
//   typst compile Documentation/SMYSL_MANUAL.typ
//
// This is the book-scale rewrite of the earlier single-file manual, built
// around the why -> real-example -> what's-next-and-why contract established
// in Documentation/manual/design.typ. Every chapter file under
// Documentation/manual/ was written and independently verified against the
// real smysl binary; this file only assembles them in table-of-contents
// order.

#import "manual/design.typ": *

#book((
  // ── Part I — Foundations ──────────────────────────────────────────────
  include "manual/ch01-03-foundations.typ",

  // ── Part II — Creating and Formatting Documents ──────────────────────
  include "manual/ch04-05-creating-formatting.typ",

  // ── Part III — Validating While You Write ────────────────────────────
  // ── Part IV — Infer and Enrich (opens here, continues below) ─────────
  include "manual/ch06-08-validate-infer.typ",
  include "manual/ch09-10-attest-providers.typ",

  // ── Part V — Operating on Documents ──────────────────────────────────
  include "manual/ch11-13-merge-diff-trace.typ",
  include "manual/ch14-16-view-salience-retract.typ",
  include "manual/ch17-18-pack-thread.typ",

  // ── Part VI — Verification ────────────────────────────────────────────
  include "manual/ch19-21-verify.typ",

  // ── Part VII — Export for Human Consumption ──────────────────────────
  include "manual/ch22-24-render.typ",

  // ── Part VIII — smysl as a Library ────────────────────────────────────
  // ── Part IX — A Complete Walkthrough (opens partway through the file) ─
  include "manual/ch25-27-library-walkthrough.typ",

  // ── Appendices ─────────────────────────────────────────────────────────
  include "manual/appendices.typ",
))
