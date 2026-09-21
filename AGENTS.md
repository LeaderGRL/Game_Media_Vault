# Development workflow

Read `CONTEXT.md` before changing domain behavior and `docs/SPEC.md` before changing product behavior. Respect accepted ADRs under `docs/adr/`.

For every feature or bug fix, use a vertical red -> green cycle. Establish the public seam first; by default, test behavior through the application/use-case boundary while replacing connector, catalog, storage, and network ports with test doubles. Add one behavior test, run it and observe the expected failure, implement only enough production code to make it pass, then repeat for the next behavior. Keep tests independent of private implementation details.

Create a commit for every coherent modification or feature after its relevant checks pass. Keep the test, implementation, and directly related documentation for one behavior in the same commit. Keep unrelated changes in separate commits.

Run the applicable local equivalents of the GitHub CI checks before committing. Treat formatting, linting, tests, and build failures as blockers.

Write all code comments in English.

