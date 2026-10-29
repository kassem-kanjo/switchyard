import { describe, expect, it } from "vitest";
import { GUIDE_KEY, GUIDE_STEPS, dismiss, isDismissed, stepLabel, type KV } from "./guideModel";

function fakeKV(initial: Record<string, string> = {}): KV & { data: Map<string, string> } {
  const data = new Map(Object.entries(initial));
  return {
    data,
    getItem: (k) => data.get(k) ?? null,
    setItem: (k, v) => {
      data.set(k, v);
    },
  };
}

const throwingKV: KV = {
  getItem() {
    throw new Error("storage blocked");
  },
  setItem() {
    throw new Error("storage blocked");
  },
};

describe("guide model", () => {
  it("lists the three guide steps", () => {
    expect(GUIDE_STEPS).toHaveLength(3);
    expect(GUIDE_STEPS[1]).toBe("Trace a late parcel");
  });

  it("is not dismissed by default", () => {
    expect(isDismissed(fakeKV())).toBe(false);
    expect(isDismissed()).toBe(false);
  });

  it("remembers a dismissal", () => {
    const kv = fakeKV();
    dismiss(kv);
    expect(kv.data.get(GUIDE_KEY)).toBe("1");
    expect(isDismissed(kv)).toBe(true);
  });

  it("treats any stored value other than \"1\" as not dismissed", () => {
    expect(isDismissed(fakeKV({ [GUIDE_KEY]: "true" }))).toBe(false);
    expect(isDismissed(fakeKV({ [GUIDE_KEY]: "0" }))).toBe(false);
    expect(isDismissed(fakeKV({ [GUIDE_KEY]: "" }))).toBe(false);
  });

  it("never throws when the store throws", () => {
    expect(() => isDismissed(throwingKV)).not.toThrow();
    expect(isDismissed(throwingKV)).toBe(false);
    expect(() => dismiss(throwingKV)).not.toThrow();
  });

  it("labels steps with circled digits and bolds only the current one", () => {
    expect(stepLabel(0, 1)).toEqual({ n: "①", text: "Incident", bold: false });
    expect(stepLabel(1, 1)).toEqual({ n: "②", text: "Trace a late parcel", bold: true });
    expect(stepLabel(2, 1)).toEqual({ n: "③", text: "Read the post-mortem", bold: false });
  });
});
