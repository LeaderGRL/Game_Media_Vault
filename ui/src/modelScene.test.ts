import { describe, expect, it, vi } from "vitest";

import { showModel } from "./modelScene";

describe("showModel", () => {
  it("reports WebGL as unavailable instead of drawing", () => {
    // jsdom offers no WebGL context, like a webview without hardware acceleration.
    vi.spyOn(console, "error").mockImplementation(() => {});
    const host = document.createElement("div");
    const onReady = vi.fn();
    const onError = vi.fn();

    const stop = showModel(host, "gmv-object://localhost/model1", { onReady, onError });

    expect(onError).toHaveBeenCalledTimes(1);
    expect(onReady).not.toHaveBeenCalled();
    expect(host.childElementCount).toBe(0);
    expect(() => stop()).not.toThrow();
  });
});
