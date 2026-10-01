import { describe, expect, it } from "vitest";

import { previewMediaType } from "./types";

const noBytes = new Uint8Array();

describe("previewMediaType", () => {
  it("derives image media types from the original filename", () => {
    expect(previewMediaType(noBytes, "Tetris (World) (Rev 1).PNG")).toBe("image/png");
    expect(previewMediaType(noBytes, "scan.jpeg")).toBe("image/jpeg");
    expect(previewMediaType(noBytes, "cover.webp")).toBe("image/webp");
    expect(previewMediaType(noBytes, "logo.svg")).toBe("image/svg+xml");
    expect(previewMediaType(noBytes, "manual.pdf")).toBe("application/octet-stream");
    expect(previewMediaType(noBytes, "no-extension")).toBe("application/octet-stream");
  });

  it("reads the media type from the preview bytes before the filename", () => {
    const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00]);
    const jpeg = new Uint8Array([0xff, 0xd8, 0xff, 0xe0]);
    const gif = new TextEncoder().encode("GIF89a");

    expect(previewMediaType(png, "renamed.jpg")).toBe("image/png");
    expect(previewMediaType(jpeg, "cover")).toBe("image/jpeg");
    expect(previewMediaType(gif, "cover.png")).toBe("image/gif");
  });
});
