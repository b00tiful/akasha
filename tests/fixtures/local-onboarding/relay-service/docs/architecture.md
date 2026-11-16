# Architecture
Relay dispatch drains the queue through the codec into process-local storage, returning metrics.
The backoff module is a policy helper; dispatch does not call it or schedule retries.
