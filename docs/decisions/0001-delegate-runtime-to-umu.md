# ADR 0001 · Delegate Windows execution to UMU

## Status

Accepted for the MVP.

## Context

drabi needs to run installers and registered Windows games while preserving the user's choice of Proton and prefix. Proton uses the runtime environment set up by UMU.

## Decision

Invoke the official UMU zipapp from the drabi data directory. Pass the executable and Windows arguments directly and set the absolute Wine and Proton paths. Use UMU's empty-target invocation for explicit prefix initialization. Check the resulting prefix structure because Proton may report a missing executable after creating the prefix. Resolve the working directory before starting the child.

Check release metadata through the official GitHub endpoint. Verify the zipapp archive against the asset's SHA-256 digest, unpack into staging, and run the launcher version check before publishing. Serialize installation with a file lock and atomically switch the active symlink. Retain installed version directories in the MVP because in-flight processes may still use a previously resolved zipapp. The latest active launcher continues to work after failed automatic update checks.

## Alternatives considered

Using the distribution's system UMU package couples availability and update timing to the host. Calling Proton directly shifts Steam Linux Runtime setup and launcher behavior into drabi. The official zipapp offers user-local installation with the upstream runtime behavior.

## References

- [UMU launcher manual](https://github.com/Open-Wine-Components/umu-launcher/blob/main/docs/umu.1.scd)
- [Official UMU releases](https://github.com/Open-Wine-Components/umu-launcher/releases)
