# Preserve original assets in a content-addressed store

Every successfully ingested original Asset is immutable and stored outside the relational catalog in a BLAKE3-addressed object store; crops, resized images, normalized files, extracted pages, textures, and generated 3D media are separate Derived Assets. This preserves archival fidelity, deduplicates identical files across providers without losing provenance, and prevents later processing changes from destroying the best source material.
