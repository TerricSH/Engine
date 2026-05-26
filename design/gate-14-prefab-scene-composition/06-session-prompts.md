# Gate 14 Session Prompts

Gate 14 creates reusable composition. Keep prefab schema versioned and avoid breaking base scene format.

## Session 14A: Prefab Runtime Owner

Goal: Implement prefab schema, loading, instantiation, nested/variant validation.

Owns:
- prefab asset/runtime modules

Must not edit:
- base scene schema in breaking ways

Expected output:
- prefab assets instantiate into ECS
- source links and instance identity preserved

Validation:
- prefab round-trip tests

## Session 14B: Override And Editor Workflow Owner

Goal: Implement override records and editor create/apply/revert workflow.

Owns:
- prefab editor plugin
- override diff/apply/revert modules

Must not edit:
- ECS internals directly

Expected output:
- overrides visible and reversible

Validation:
- editor prefab workflow test

## Session 14C: Archetypes And Pooling Owner

Goal: Implement curated archetypes and object pooling lifecycle.

Owns:
- archetype registry
- pooling manager

Must not edit:
- physics/animation internals

Expected output:
- validation scene uses archetypes
- pooled entities reset safely

Validation:
- pool lifecycle tests
