# Relay service
A synthetic in-memory message relay for onboarding acceptance. Configuration, deduplicating
queue, codec, retry policy, storage, dispatch and metrics are separate modules. No network
listener or background retry worker is implemented. Run scripts/check.sh to verify behavior.
