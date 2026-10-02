import {
  SourceDescription,
  SourceFailure,
  SourceFailureSummary,
  assetTypeLabel,
  sourceLabel,
} from "./acquisition";

interface SourcesViewProps {
  /** The registered Sources, or `null` while they are read or after reading them failed. */
  sources: SourceDescription[] | null;
  /** Why the Sources could not be read, if they could not. */
  error?: string | null;
  /** The failures the loaded vault recorded by Source; `null` without a vault or a reading. */
  failures?: SourceFailureSummary[] | null;
}

/** The registered Sources, described from the capabilities planning uses. */
export function SourcesView({ sources, error = null, failures = null }: SourcesViewProps) {
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
        const summary = failures?.find((failure) => failure.source_id === source.source_id);
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
              {failures !== null ? (
                <>
                  <dt>Recent failures</dt>
                  <dd>
                    {summary === undefined ? (
                      "No failure recorded"
                    ) : (
                      <>
                        <span>
                          {summary.failures}{" "}
                          {summary.failures === 1 ? "failure recorded" : "failures recorded"}
                        </span>
                        <ul className="source-failures">
                          {summary.latest.map((failure) => (
                            <li key={failure.sequence}>{describeFailure(failure)}</li>
                          ))}
                        </ul>
                      </>
                    )}
                  </dd>
                </>
              ) : null}
            </dl>
          </section>
        );
      })}
    </div>
  );
}

function describeFailure(failure: SourceFailure) {
  // Recorded times read the same on every machine.
  const recorded = new Date(failure.recorded_at * 1000).toISOString().slice(0, 16);
  return `${recorded.replace("T", " ")} UTC · ${failure.stage} · run ${failure.run_id}: ${failure.message}`;
}
