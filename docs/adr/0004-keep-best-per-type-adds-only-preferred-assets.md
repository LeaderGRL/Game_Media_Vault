# Keep Best Per Type adds only Preferred Assets and never removes retained ones

The Preferred Asset of a Release Edition and Asset Type is derived whenever the library is read, from the Assets that provenance still retains: the original with the most pixels wins (an unknown pixel size ranks below every known one), then the one with more bytes at the same pixel count, then the one acquired first. Nothing about preference is stored, so a change to the ranking or to the retained Assets is reflected on the next read, and every other Asset of the type carries the reason it ranks lower.

A run with the Keep Best Per Type Retention Policy links an acquired original only if it would become the Preferred Asset. The retained Assets are compared in the same transaction as the link, so two concurrent runs cannot both add an inferior original. An original that a retained Asset outranks is not linked: the candidate is settled as for an original below the quality requirements (its Review Item is closed automatically and work parked on it is requeued, see ADR 0003), and its work records which Asset outranks it and why, so the alternative stays explainable and its candidate metadata available for a later run. Bytes that a retained Asset already holds are that Asset, so they are linked as more provenance, never outranked.

Keep Best Per Type never detaches Assets that other runs, other policies or local imports retained, including ones a new original now outranks; they stay in the library and simply stop being preferred. Within one run, candidates for the same Release Edition and type are compared in processing order against what is retained at that moment.

## Considered options

- Storing a preferred flag per Asset. Rejected: it duplicates what the ranking derives, and every link, detachment or ranking change would have to keep it consistent.
- Letting Keep Best Per Type detach the Assets it outranks. Rejected for now: it would silently drop originals that another policy retained on purpose, and originals are what the vault preserves. Pruning outranked Assets can become an explicit, reviewable vault action later.
- Request-specific preferences (`preferred_scan_type`, `preferred_source_priority`, `best_available`). Not applied yet: a derived Preferred Asset has no request to read them from, so they need a configurable scoring policy shared by the library and acquisition. Requests using them are still refused before discovery.
