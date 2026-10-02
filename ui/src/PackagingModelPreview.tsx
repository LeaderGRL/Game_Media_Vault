import { useEffect, useRef, useState } from "react";

interface PackagingModelPreviewProps {
  /** URL of the glTF binary model. */
  url: string;
  label: string;
}

/**
 * An interactive 3D preview of a packaging model. The 3D engine loads only once a model is
 * shown, and the Library goes on without the preview when WebGL or the model is unavailable.
 */
export function PackagingModelPreview({ url, label }: PackagingModelPreviewProps) {
  const host = useRef<HTMLDivElement>(null);
  const [state, setState] = useState<"loading" | "ready" | "unavailable">("loading");

  useEffect(() => {
    const element = host.current;
    if (element === null) {
      return;
    }
    let cancelled = false;
    let stop: (() => void) | null = null;
    setState("loading");
    import("./modelScene")
      .then(({ showModel }) => {
        if (cancelled) {
          return;
        }
        stop = showModel(element, url, {
          onReady: () => {
            if (!cancelled) {
              setState("ready");
            }
          },
          onError: () => {
            if (!cancelled) {
              setState("unavailable");
            }
          },
        });
      })
      .catch(() => {
        if (!cancelled) {
          setState("unavailable");
        }
      });
    return () => {
      cancelled = true;
      stop?.();
    };
  }, [url]);

  return (
    <figure className="packaging-model" aria-label={label}>
      <div className="packaging-model-scene" ref={host} />
      <figcaption>
        {state === "unavailable"
          ? "3D preview unavailable"
          : state === "loading"
            ? "Loading the 3D box…"
            : "Drag to turn the box"}
      </figcaption>
    </figure>
  );
}
