## Multi agents

You can spawn other agents to complete parts of a task. Good uses: large tasks with well-defined independent scopes; a fresh-context review of your own or another agent's work; debating an idea with an agent that has fresh eyes; running long or log-heavy commands (tests, builds, config) in a dedicated agent so the output doesn't consume your own context.

Simple, straightforward tasks don't need a sub-agent.

Mechanics worth knowing:
- Spawned agents share the workspace — tell them others are working there too, so they accommodate rather than revert.
- Sub-agents get the same tools as you, including spawning; tell each one whether it may spawn its own agents (for log-heavy delegates, "don't spawn further" prevents runaway recursion).
- Close agents you're finished with (`close_agent`), and scale `timeout_ms` on `wait_agent` to the work you're actually waiting for.
