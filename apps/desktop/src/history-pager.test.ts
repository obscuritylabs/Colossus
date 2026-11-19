import { describe, expect, it } from "vitest";
import { HistoryPager } from "./history-pager";

function start(pager: HistoryPager, next = "older") {
  const request = pager.begin("", true)!;
  pager.accept(request, next, true);
  pager.finish(request);
}

describe("HistoryPager", () => {
  it("admits one page synchronously across observer, retry and manual events", () => {
    const pager = new HistoryPager();
    start(pager);
    const page = pager.begin("older", false, true)!;
    expect(page).not.toBeNull();
    pager.resume();
    expect(pager.begin("older", false)).toBeNull();
    expect(pager.begin("", true)).toBeNull();
    expect(pager.finish(page)).toBe(true);
  });

  it("invalidates stale success, error and finally after a scope replacement", () => {
    const pager = new HistoryPager();
    start(pager);
    const old = pager.begin("older", false)!;
    pager.reset();
    const fresh = pager.begin("", true)!;
    expect(pager.isCurrent(old)).toBe(false);
    expect(pager.accept(old, "stale", true)).toBe("");
    pager.fail(old);
    expect(pager.finish(old)).toBe(false);
    expect(pager.isCurrent(fresh)).toBe(true);
    pager.accept(fresh, "fresh", true);
    pager.finish(fresh);
    expect(pager.begin("stale", false)).toBeNull();
    expect(pager.begin("fresh", false)).not.toBeNull();
  });

  it("retains the failed cursor and suppresses automatic retries", () => {
    const pager = new HistoryPager();
    start(pager);
    const page = pager.begin("older", false)!;
    pager.fail(page);
    pager.finish(page);
    expect(pager.begin("older", false, true)).toBeNull();
    pager.resume();
    expect(pager.begin("skipped", false)).toBeNull();
    expect(pager.begin("older", false)).not.toBeNull();
  });

  it("permits advancing empty pages within a finite no-progress budget", () => {
    const pager = new HistoryPager();
    start(pager, "page-0");
    for (let index = 0; index < 8; index += 1) {
      const request = pager.begin(`page-${index}`, false, true)!;
      const notice = pager.accept(request, `page-${index + 1}`, false);
      expect(notice !== "").toBe(index === 7);
      pager.finish(request);
    }
    expect(pager.begin("page-8", false, true)).toBeNull();
    pager.resume();
    expect(pager.begin("page-8", false)).not.toBeNull();
  });

  it("pauses cyclic cursors and does not invent an end for a short page", () => {
    const pager = new HistoryPager();
    start(pager, "a");
    const a = pager.begin("a", false)!;
    expect(pager.accept(a, "b", false)).toBe("");
    pager.finish(a);
    const b = pager.begin("b", false)!;
    expect(pager.accept(b, "a", false)).toContain("Refresh threads");
    pager.finish(b);
    expect(pager.begin("a", false, true)).toBeNull();
    const fresh = pager.begin("", true)!;
    pager.accept(fresh, "", true);
    pager.finish(fresh);
    expect(pager.begin("", false)).toBeNull();
  });

  it("yields a finite successful automatic burst until explicit continuation", () => {
    const pager = new HistoryPager();
    start(pager, "p0");
    for (let index = 0; index < 16; index += 1) {
      const request = pager.begin(`p${index}`, false, true)!;
      const notice = pager.accept(request, `p${index + 1}`, true);
      expect(notice !== "").toBe(index === 15);
      pager.finish(request);
    }
    expect(pager.begin("p16", false, true)).toBeNull();
    pager.resume();
    expect(pager.begin("p16", false)).not.toBeNull();
  });
});
