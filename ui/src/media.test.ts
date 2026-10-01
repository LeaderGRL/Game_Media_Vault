import { describe, expect, it } from "vitest";

import { previewMediaType } from "./types";

const noBytes = new Uint8Array();

describe("previewMediaType", () => {
  it("derives image media types from the original filename", () => {
    expect(previewMediaType(noBytes, "Tetris (World) (Rev 1).PNG")).toBe("image/png");
    expect(previewMediaType(noBytes, "scan.jpeg")).toBe("image/jpeg");
    expect(previewMediaType(noBytes, "cover.webp")).toBe("image/webp");
    // SVG can carry script, so untrusted SVG previews stay inert bytes.
    expect(previewMediaType(noBytes, "logo.svg")).toBe("application/octet-stream");
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

  it("recognizes AVIF bytes by their ISO-BMFF brand", () => {
    const avif = new Uint8Array([0, 0, 0, 0x1c, ...new TextEncoder().encode("ftypavif")]);
    const sequence = new Uint8Array([0, 0, 0, 0x1c, ...new TextEncoder().encode("ftypavis")]);

    expect(previewMediaType(avif, "cover.jpg")).toBe("image/avif");
    expect(previewMediaType(sequence, "cover.jpg")).toBe("image/avif");
  });

  it("recognizes AVIF in an extended-size ftyp box", () => {
    // size 1, then a 64-bit largesize of 32, major brand avif, minor version, brand mif1.
    const extended = new Uint8Array([
      0, 0, 0, 1, ...new TextEncoder().encode("ftyp"), 0, 0, 0, 0, 0, 0, 0, 32,
      ...new TextEncoder().encode("avif"), 0, 0, 0, 0, ...new TextEncoder().encode("mif1"),
    ]);

    expect(previewMediaType(extended, "cover.jpg")).toBe("image/avif");
  });

  it("recognizes AVIF declared among the compatible brands", () => {
    // ftyp box: size 24, major brand mif1, minor version, compatible brands miaf and avif.
    const ftyp = new Uint8Array([
      0, 0, 0, 24, ...new TextEncoder().encode("ftypmif1"), 0, 0, 0, 0,
      ...new TextEncoder().encode("miafavif"),
    ]);
    const heic = new Uint8Array([
      0, 0, 0, 20, ...new TextEncoder().encode("ftypmif1"), 0, 0, 0, 0,
      ...new TextEncoder().encode("heic"), ...new TextEncoder().encode("avif"),
    ]);

    expect(previewMediaType(ftyp, "cover.jpg")).toBe("image/avif");
    // Brands past the declared box size belong to the next box.
    expect(previewMediaType(heic, "cover.jpg")).toBe("image/jpeg");
  });
});
