import { SourceDescription, assetTypeLabel, sourceLabel } from "./acquisition";

interface SourcesViewProps {
  /** The registered Sources, or `null` while they are read or after reading them failed. */
  sources: SourceDescription[] | null;
  /** Why the Sources could not be read, if they could not. */
  error?: string | null;
}

/** The registered Sources, described from the capabilities planning uses. */
export function SourcesView({ sources, error = null }: SourcesViewProps) {
  if (error !== null) {
    return (
      <p className="error-message" role="alert">
        {error}
      </p>
    );
  }
  if (sources === null) {
    return <p className="hint">Reading the registered Sources…</p>;
  }
  return (
    <div className="sources">
      {sources.map((source) => {
        const name = sourceLabel(source.source_id);
        return (
          <section className="source" aria-label={name} key={source.source_id}>
            <h2>{name}</h2>
            <dl>
              <dt>Acquires</dt>
              <dd>{source.asset_types.map(assetTypeLabel).join(", ")}</dd>
              <dt>Acquisition method</dt>
              <dd>
                {source.direct_media_download
                  ? "Downloads media directly"
                  : "Cannot download media directly"}
              </dd>
            </dl>
          </section>
        );
      })}
    </div>
  );
}
