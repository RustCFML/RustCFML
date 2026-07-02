export const meta = {
  name: 'websocket-fluent-design-survey',
  description: 'Survey top WebSocket/realtime APIs across ecosystems, distill fluent-design principles, critique the RustCFML design',
  phases: [
    { title: 'Survey', detail: 'one agent per ecosystem cluster, extract fluent-design principles with citations' },
    { title: 'Distill', detail: 'dedup + rank principles across all findings' },
    { title: 'Critique', detail: 'adversarially evaluate the RustCFML design through 3 lenses' },
    { title: 'Synthesize', detail: 'draft refined principles section + concrete design deltas' },
  ],
}

const FINDINGS = {
  type: 'object',
  required: ['framework', 'language', 'api_paradigm', 'principles', 'standout_features', 'pain_points', 'citations'],
  properties: {
    framework: { type: 'string' },
    language: { type: 'string' },
    api_paradigm: { type: 'string', description: 'e.g. annotation/decorator, convention callbacks, actor, builder-chain, pub/sub channels, hub-RPC, reactive streams' },
    principles: {
      type: 'array',
      items: {
        type: 'object',
        required: ['name', 'what', 'example_code', 'why_pleasant', 'applicability_to_cfml'],
        properties: {
          name: { type: 'string', description: 'short principle name' },
          what: { type: 'string' },
          example_code: { type: 'string', description: 'a small real code snippet demonstrating it' },
          why_pleasant: { type: 'string', description: 'what makes this fluent/elegant for developers' },
          applicability_to_cfml: { type: 'string', description: 'could this translate to a CFML/RustCFML interface? how, or why not' },
        },
      },
    },
    standout_features: { type: 'array', items: { type: 'string' } },
    pain_points: { type: 'array', items: { type: 'string' }, description: 'criticisms / friction reported for this API' },
    citations: { type: 'array', items: { type: 'object', required: ['title', 'url'], properties: { title: { type: 'string' }, url: { type: 'string' } } } },
  },
}

const PRINCIPLES = {
  type: 'object',
  required: ['principles'],
  properties: {
    principles: {
      type: 'array',
      items: {
        type: 'object',
        required: ['principle', 'summary', 'prevalence', 'frameworks_exhibiting', 'dx_impact', 'rustcfml_application', 'code_sketch'],
        properties: {
          principle: { type: 'string' },
          summary: { type: 'string' },
          prevalence: { type: 'integer', description: 'how many surveyed frameworks exhibit this' },
          frameworks_exhibiting: { type: 'array', items: { type: 'string' } },
          dx_impact: { type: 'string', enum: ['high', 'medium', 'low'] },
          rustcfml_application: { type: 'string', description: 'concretely how it would shape the RustCFML CFML API or architecture' },
          code_sketch: { type: 'string', description: 'a CFML snippet showing the principle applied to our proposed API' },
          tension: { type: 'string', description: 'any conflict with CFML idioms or our current design, if any' },
        },
      },
    },
  },
}

const CRITIQUE = {
  type: 'object',
  required: ['lens', 'gaps', 'things_we_got_right', 'over_engineering', 'verdict'],
  properties: {
    lens: { type: 'string' },
    gaps: {
      type: 'array',
      items: {
        type: 'object',
        required: ['principle', 'our_current_state', 'recommended_change', 'severity', 'cfml_sketch'],
        properties: {
          principle: { type: 'string' },
          our_current_state: { type: 'string' },
          recommended_change: { type: 'string' },
          severity: { type: 'string', enum: ['critical', 'high', 'medium', 'nice-to-have'] },
          cfml_sketch: { type: 'string' },
        },
      },
    },
    things_we_got_right: { type: 'array', items: { type: 'string' } },
    over_engineering: { type: 'array', items: { type: 'string' }, description: 'parts of our design that are heavier than peers and could be simplified' },
    verdict: { type: 'string' },
  },
}

const CLUSTERS = [
  { key: 'nodejs-socketio', title: 'Node.js: socket.io (server + client API), ws, Primus, Engine.IO' },
  { key: 'elixir-phoenix', title: 'Elixir: Phoenix Channels, Phoenix.Presence, and LiveView declarative server-rendered realtime' },
  { key: 'rails-actioncable', title: 'Ruby on Rails: Action Cable (channels/streams/broadcasting) and Hotwire Turbo Streams' },
  { key: 'dotnet-signalr', title: 'ASP.NET Core: SignalR — hubs, strongly-typed Hub<T> interfaces, groups, client streaming, MessagePack' },
  { key: 'go-stack', title: 'Go: gorilla/websocket, coder/nhooyr websocket, melody, gobwas; plus the Centrifugo/centrifuge realtime server' },
  { key: 'python-stack', title: 'Python: Starlette/FastAPI WebSocket routes, Django Channels async consumers + groups, python-socketio' },
  { key: 'jvm-stack', title: 'JVM: Spring WebSocket + STOMP messaging (@MessageMapping/@SendTo), Ktor websockets DSL, Javalin, Vert.x event bus, Akka HTTP' },
  { key: 'rust-stack', title: 'Rust: socketioxide (extractors), axum::extract::ws, tokio-tungstenite, actix-web actors, salvo — async websocket ergonomics' },
  { key: 'php-laravel', title: 'PHP: Laravel Reverb/Echo/Broadcasting events + Pusher protocol, Ratchet, Swoole/Workerman, Symfony Mercure' },
  { key: 'managed-services', title: 'Managed realtime services: Pusher, Ably, Supabase Realtime, PubNub, AWS API Gateway WebSockets — SDK ergonomics, channel/presence design' },
  { key: 'edge-runtime', title: 'Edge/runtime-native: Cloudflare Durable Objects WebSocket Hibernation API + PartyKit, Bun.serve websockets, Deno, Hono/Elysia' },
  { key: 'typed-rpc-sync', title: 'Typed/declarative realtime: GraphQL subscriptions, tRPC subscriptions, gRPC streaming, Convex/Replicache/Liveblocks sync engines — and cross-cutting design of presence, reconnection, backpressure, acks' },
]

const surveyPrompt = (c) => `You are researching the developer-facing API design of a realtime/WebSocket implementation, to extract FLUENT DESIGN PRINCIPLES that make it pleasant to use. This feeds a design review for a new WebSocket API in a CFML interpreter (RustCFML).

CLUSTER TO RESEARCH: ${c.title}

Use web search and fetch authoritative docs/READMEs/guides for each item in the cluster. Do real research — run several searches, open the official docs and a couple of well-regarded tutorials/critiques. Do NOT rely only on memory.

For EACH notable framework/library in the cluster, study how a developer actually writes a realtime handler and extract what is ELEGANT about the interface. Focus on DESIGN PRINCIPLES, not feature lists. Pay specific attention to:
- How handlers are DECLARED: annotations/decorators, macros, convention method names, builder/DSL, actors, registration callbacks. What reads cleanly?
- Naming and verbs (emit/broadcast/publish/push/send/to/in/join), and how channels/rooms/topics/namespaces are modeled.
- Declarative vs imperative wiring. Zero-boilerplate setup. Co-location of routing with handler.
- "Emit from anywhere" (broadcasting outside a connection context) ergonomics.
- Type-safety / schema / payload contracts (strongly-typed hubs, typed events, codecs).
- Presence, reconnection, resumability, acknowledgements, request/response over a socket.
- Backpressure, lifecycle/cleanup, error handling ergonomics.
- Client-side API pairing (what makes the JS/client side pleasant) where relevant.
- Testing ergonomics.

Return concrete small code snippets for each principle. Be critical: also record PAIN POINTS / criticisms developers report. For each principle, judge whether it could translate to a CFML/RustCFML interface (CFML = dynamically typed, case-insensitive, component/CFC based, has function & component attribute annotations and getMetadata reflection, no generics).

Return ONE FINDINGS object covering the most important framework(s) in the cluster (you may set framework to a slash-joined list if covering several). Prioritize depth on the single most influential one.`

const distillPrompt = (findings) => `You are distilling FLUENT DESIGN PRINCIPLES for realtime/WebSocket APIs from a multi-ecosystem survey. Below is the raw survey JSON from ~12 ecosystem clusters.

Produce a DEDUPLICATED, RANKED catalog of the cross-cutting principles. Merge principles that are the same idea under different names. For each: count prevalence (how many distinct frameworks exhibit it), list which frameworks, rate dx_impact, and — most importantly — describe concretely how it would shape the RustCFML CFML-facing API or Rust architecture, with a CFML code sketch using our proposed API shape (component socket="/chat", on="event" function attributes, socket.emit/broadcast/join/to, io() ambient accessor). Note any tension with CFML idioms or our current design.

Rank by (dx_impact, prevalence). Include both the obvious high-prevalence principles AND the rare-but-brilliant ones worth stealing. Aim for 15-25 principles. Be specific and opinionated.

RAW SURVEY JSON:
${JSON.stringify(findings)}`

const LENSES = [
  { key: 'dx-ergonomics', title: 'Developer experience & fluency', focus: 'Is our proposed CFML API the most pleasant it could be? Judge naming, the on="event" annotation vs alternatives, the socket object surface, the io() ambient accessor, discovery/config, zero-boilerplate. What fluent principles from the survey are we MISSING? What would a Phoenix/SignalR/socket.io developer find clunky here?' },
  { key: 'protocol-correctness', title: 'Protocol completeness & correctness', focus: 'Judge whether the design covers the realtime concerns that mature implementations consider table-stakes: acknowledgements/request-response over socket, presence, reconnection/resumability, backpressure, binary frames, heartbeats, auth at handshake, error propagation, graceful close. What is absent or under-specified vs peers?' },
  { key: 'cfml-fit-simplicity', title: 'CFML-idiom fit & over-engineering', focus: 'Judge whether the design is idiomatic CFML and whether any part is heavier than peer frameworks. Is the convention+annotation hybrid the right call vs a single consistent mechanism? Is the unified raw-WS/socket.io socket object honest or leaky? Is anything (RoomAdapter, dual transports) premature? What would a Lucee/BoxLang/Wheels developer expect?' },
]

const critiquePrompt = (lens, distilled, design) => `You are an adversarial design reviewer for the RustCFML WebSocket API. Evaluate it through ONE lens: ${lens.title}.

LENS FOCUS: ${lens.focus}

Be skeptical and concrete. Default to finding real gaps rather than praising. Ground every gap in a principle observed in mature implementations (Phoenix, SignalR, socket.io, Action Cable, Centrifugo, etc.). For each gap, give the recommended change and a CFML code sketch of the improved API. Also note what we genuinely got right, and call out any OVER-ENGINEERING (parts heavier than peers).

OUR CURRENT DESIGN:
${design}

DISTILLED CROSS-ECOSYSTEM PRINCIPLES (ranked):
${JSON.stringify(distilled)}`

const synthPrompt = (distilled, critiques, design) => `You are the lead designer integrating a cross-ecosystem fluent-design review into the RustCFML WebSocket design doc.

Using the distilled principles and the three critiques below, write TWO markdown sections ready to paste into the design doc:

1. "## Fluent design principles (cross-ecosystem)" — a tight, opinionated catalog of the principles worth adopting, each with: the principle, where it comes from (name 2-4 frameworks), and a CFML code sketch applying it to OUR API. Group/order by impact. Be concrete, not encyclopedic.

2. "## Refinements to the proposed design" — a prioritized list of concrete CHANGES to our current design (additions, renames, reconsiderations), each tagged [adopt]/[consider]/[defer] with a one-line rationale and, where it changes the API, a before/after CFML sketch. Resolve the open questions (binary frames; static-like scope vs set/get; handshake auth hook) with a recommendation. Flag any peer principle we should deliberately NOT adopt and why.

Write in the same crisp register as the existing doc. Markdown only, no preamble.

OUR CURRENT DESIGN:
${design}

DISTILLED PRINCIPLES:
${JSON.stringify(distilled)}

CRITIQUES:
${JSON.stringify(critiques)}`

phase('Survey')
const findings = (await parallel(CLUSTERS.map(c => () =>
  agent(surveyPrompt(c), { label: `survey:${c.key}`, phase: 'Survey', schema: FINDINGS })
))).filter(Boolean)
log(`Surveyed ${findings.length}/${CLUSTERS.length} ecosystem clusters`)

phase('Distill')
let distilled = null
for (let attempt = 0; attempt < 5 && !distilled; attempt++) {
  distilled = await agent(distillPrompt(findings), {
    label: attempt === 0 ? 'distill-principles' : `distill-principles-retry${attempt}`,
    phase: 'Distill',
    schema: PRINCIPLES,
  })
}
if (!distilled || !Array.isArray(distilled.principles)) { distilled = { principles: [] } }
log(`Distilled ${distilled.principles.length} cross-ecosystem principles`)

phase('Critique')
const critiques = (await parallel(LENSES.map(l => () =>
  agent(critiquePrompt(l, distilled, args.ourDesign), { label: `critique:${l.key}`, phase: 'Critique', schema: CRITIQUE })
))).filter(Boolean)
log(`Completed ${critiques.length}/${LENSES.length} adversarial critiques`)

phase('Synthesize')
const draft = await agent(synthPrompt(distilled, critiques, args.ourDesign), { label: 'synthesize-doc-sections', phase: 'Synthesize' })

return { principles: distilled.principles, critiques, draft, raw_findings: findings }