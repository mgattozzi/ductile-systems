# ductile-systems

## Rules

1. All comments must be authored by a human
2. All dependencies must be listed as a workspace dependency in the root Cargo.toml
3. All imports must be at the top of the module
4. Do not use fully qualified paths (e.g. std::error::Error) in code, instead import them with a
   use statement
5. When adding tests consider why the test is being added. Does it provide value? Does it actually
   do anything? How useful is this test actually? Lots of tests is not always good. Sometimes
   focused and simple tests cover the majority of failures or invariants we want to uphold.
6. Commits should be atomic in nature and contain cohesive changes

## Tool Uses

1. If available and the user has initialize the repo to do so use jj instead of git
