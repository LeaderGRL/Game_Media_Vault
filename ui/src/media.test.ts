import { describe, expect, it } from "vitest";

import { previewMediaType } from "./types";

describe("previewMediaType", () => {
  it("derives image media types from the original filename", () => {
    expect(previewMediaType("Tetris (World) (Rev 1).PNG")).toBe("image/png");
    expect(previewMediaType("scan.jpeg")).toBe("image/jpeg");
    expect(previewMediaType("cover.webp")).toBe("image/webp");
    expect(previewMediaType("manual.pdf")).toBe("application/octet-stream");
    expect(previewMediaType("no-extension")).toBe("application/octet-stream");
  });
});
