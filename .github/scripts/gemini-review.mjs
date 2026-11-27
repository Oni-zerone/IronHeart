// Reviews a PR with a single Gemini call: only the diff in the prompt (with its context lines), a JSON answer, and a
// review published through the GitHub API. No agent and no tools: the cost is proportional to the diff.
// After the first review, later pushes only get the new commits reviewed (FULL=true to redo the whole PR).
// In the same call the model re-evaluates the conversations opened by its previous reviews (current code and the
// author's replies): those it accepts as fixed are closed with a reply. With EVENT pull_request_review_comment
// (a reply in a thread) only the conversations are evaluated, without a new review.
import { appendFileSync, existsSync, readFileSync } from "node:fs";

const { GITHUB_TOKEN, GEMINI_API_KEY, REPOSITORY, PR, FULL, EVENT, GITHUB_STEP_SUMMARY } = process.env;
const MODEL = process.env.GEMINI_MODEL || "gemini-3.8-flash";

// Diff limit in characters (~4 per token)
const MAX_DIFF_CHARS = 400_000;

// Files not worth having the model read
const SKIP = [
  /(^|\/)(pnpm-lock\.yaml|package-lock\.json|yarn\.lock)$/,
  /(^|\/)Cargo\.lock$/,
  /\.(png|jpe?g|gif|webp|ico|svg|woff2?|ttf|otf|pdf)$/i,
];

// Project rules (CLAUDE.md at the repository root, included by the sparse checkout): the model checks the diff
// against them
const RULES = existsSync("CLAUDE.md") ? readFileSync("CLAUDE.md", "utf8") : "";

const SEVERITIES = ["CRITICAL", "HIGH", "MEDIUM", "LOW"];

// Author of the reviews (REST and GraphQL login) and first comment of a thread opened by this script
const BOT = new Set(["github-actions[bot]", "github-actions"]);
const GEMINI_COMMENT = /^\*\*\[(CRITICAL|HIGH|MEDIUM|LOW)\]\*\*/;

async function github(path, { method = "GET", body, accept = "application/vnd.github+json" } = {}) {
  const res = await fetch(`https://api.github.com${path}`, {
    method,
    headers: {
      Authorization: `Bearer ${GITHUB_TOKEN}`,
      Accept: accept,
      "X-GitHub-Api-Version": "2022-11-28",
      ...(body && { "Content-Type": "application/json" }),
    },
    body: body && JSON.stringify(body),
  });
  if (!res.ok) {
    const error = new Error(`${method} ${path}: ${res.status} ${await res.text()}`);
    error.status = res.status;
    throw error;
  }
  return accept.includes("raw") ? res.text() : res.json();
}

async function graphql(query, variables) {
  const res = await github("/graphql", { method: "POST", body: { query, variables } });
  // GraphQL answers 200 even with errors
  if (res.errors) throw new Error(`GraphQL: ${JSON.stringify(res.errors)}`);
  return res.data;
}

async function paginate(path) {
  const items = [];
  for (let page = 1; ; page++) {
    const batch = await github(`${path}${path.includes("?") ? "&" : "?"}per_page=100&page=${page}`);
    items.push(...batch);
    if (batch.length < 100) return items;
  }
}

// Patch with the new file's line number in front of each line, plus the set of lines that can be commented on
function annotate(patch) {
  const lines = new Set();
  let next = 0;
  const text = patch
    .split("\n")
    .map((row) => {
      const hunk = row.match(/^@@ -\d+(?:,\d+)? \+(\d+)/);
      if (hunk) {
        next = Number(hunk[1]);
        return row;
      }
      if (row.startsWith("-")) return `      ${row}`;
      // An empty context line is " ": "" is only the end of the patch
      if (row.startsWith("\\") || row === "") return row;
      lines.add(next);
      return `${String(next++).padStart(5)} ${row}`;
    })
    .join("\n");
  return { text, lines };
}

function summary(text) {
  console.log(text);
  if (GITHUB_STEP_SUMMARY) appendFileSync(GITHUB_STEP_SUMMARY, `${text}\n`);
}

// Streamed answer: one SSE event ("data: {...}") per chunk. The text is the concatenation of the parts, without
// the reasoning ones (they must not end up in the JSON); token usage arrives with the last chunk.
async function readStream(res) {
  const decoder = new TextDecoder();
  let buffer = "";
  let text = "";
  let usage = {};
  let last = null;
  const handle = (event) => {
    for (const line of event.split("\n")) {
      if (!line.startsWith("data:")) continue;
      last = JSON.parse(line.slice(5));
      for (const part of last.candidates?.[0]?.content?.parts ?? []) if (!part.thought) text += part.text ?? "";
      if (last.usageMetadata) usage = last.usageMetadata;
    }
  };
  for await (const chunk of res.body) {
    buffer += decoder.decode(chunk, { stream: true }).replace(/\r\n/g, "\n");
    for (let end; (end = buffer.indexOf("\n\n")) >= 0; buffer = buffer.slice(end + 2)) handle(buffer.slice(0, end));
  }
  handle(buffer + decoder.decode());
  return { text, usage, last };
}

async function gemini(prompt) {
  const body = {
    systemInstruction: {
      parts: [
        {
          text: `You are a senior code reviewer for IronHeart, a heartbeat monitoring service: a Rust server (Axum, sqlx
with the Any driver on both SQLite and Postgres, tokio) and a Vue 3 + TypeScript dashboard (Vite, no component library).
You receive the diff of a pull request, with a few lines of context around each change. Write in English.
Of the rest of the files you only see those lines: do not report as missing something that may live outside the diff.
Report only concrete problems introduced or touched by the diff: bugs, security, data loss or inconsistency,
concurrency (locks held across await, blocking work on the async runtime, races), type errors, regressions, missing
error handling, obvious performance problems, and violations of the project rules below (for example SQL that only
works on one of the two databases, a heartbeat response other than 204, database access during a heartbeat request,
text not in English). No compliments, no style remarks or personal preferences, no repetitions. If you find no
problems, leave the list empty.
Each comment gives path and line: line is the number shown to the left of an added or context line in the diff
(never a removed line). If you propose a fix, suggestion holds the code that replaces the lines from startLine
(optional, for several lines) to line, with the correct indentation and nothing else.
severity: CRITICAL (certain damage or security hole), HIGH (likely bug), MEDIUM (edge case or fragility), LOW (minor).
summary: 2-4 sentences on what the PR does and the overall judgement.
You may also receive the conversations opened by your previous reviews (T1, T2…), with the current code and the
author's replies. For each one put id and resolved in threads: true if the code now fixes the problem (even in a
different way from the proposed fix) or if a reply convincingly explains why it is not a problem or is intentional;
false if the problem remains. reply: one or two sentences explaining the decision, addressed to the author; if
resolved is false and nobody replied, leave it empty. Do not repeat in comments the problems of conversations that
are still open.
This review is generated by ${MODEL}. Model names and versions of libraries, actions and APIs may be newer than your
knowledge: do not report them as nonexistent.${RULES ? `\n\n# Project rules (CLAUDE.md)\n\n${RULES}` : ""}`,
        },
      ],
    },
    contents: [{ role: "user", parts: [{ text: prompt }] }],
    generationConfig: {
      // when streaming, reasoning summaries arrive too: they keep the connection alive while the model thinks
      thinkingConfig: { includeThoughts: true },
      responseMimeType: "application/json",
      responseSchema: {
        type: "OBJECT",
        properties: {
          summary: { type: "STRING" },
          comments: {
            type: "ARRAY",
            items: {
              type: "OBJECT",
              properties: {
                path: { type: "STRING" },
                line: { type: "INTEGER" },
                startLine: { type: "INTEGER" },
                severity: { type: "STRING", enum: SEVERITIES },
                body: { type: "STRING" },
                suggestion: { type: "STRING" },
              },
              required: ["path", "line", "severity", "body"],
            },
          },
          threads: {
            type: "ARRAY",
            items: {
              type: "OBJECT",
              properties: {
                id: { type: "STRING" },
                resolved: { type: "BOOLEAN" },
                reply: { type: "STRING" },
              },
              required: ["id", "resolved"],
            },
          },
        },
        required: ["summary", "comments"],
      },
    },
  };

  // Quota exhausted or model overloaded: a few attempts with increasing waits
  for (let attempt = 1; ; attempt++) {
    // Streaming (SSE): with generateContent the headers only arrive once the answer is complete, and fetch gives up
    // after 5 minutes without headers (UND_ERR_HEADERS_TIMEOUT) when the diff is large.
    const res = await fetch(`https://generativelanguage.googleapis.com/v1beta/models/${MODEL}:streamGenerateContent?alt=sse`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "x-goog-api-key": GEMINI_API_KEY },
      body: JSON.stringify(body),
    });
    if (res.ok) {
      const { text, usage, last } = await readStream(res);
      if (!text) throw new Error(`Empty answer: ${JSON.stringify(last).slice(0, 2000)}`);
      return { result: JSON.parse(text), usage };
    }
    const error = `Gemini ${res.status}: ${(await res.text()).slice(0, 2000)}`;
    if (![429, 500, 503].includes(res.status) || attempt === 4) throw new Error(error);
    console.log(`::warning::${error} — retrying`);
    await new Promise((resolve) => setTimeout(resolve, attempt * 20_000));
  }
}

const repo = `/repos/${REPOSITORY}`;
const pr = await github(`${repo}/pulls/${PR}`);
const head = pr.head.sha;
const prFiles = await paginate(`${repo}/pulls/${PR}/files`);

// Lines that can be commented on: those of the whole PR's diff, the only one on which GitHub accepts comments
const commentable = new Map(prFiles.filter((f) => f.patch).map((f) => [f.filename, annotate(f.patch).lines]));

// Last commit already reviewed by this workflow: if history was not rewritten, only the new part is looked at
const reviews = await paginate(`${repo}/pulls/${PR}/reviews`);
const last = reviews
  .filter((r) => r.user?.login === "github-actions[bot]" && /Gemini review/.test(r.body ?? ""))
  .at(-1)?.commit_id;
// A reply in a thread: no new review, only the conversations with replies from the author
const threadsOnly = EVENT === "pull_request_review_comment";
let files = prFiles;
let mode = "full";
if (threadsOnly) {
  files = [];
  mode = "conversations only";
} else if (FULL !== "true" && last) {
  if (last === head) {
    files = [];
    mode = "conversations only";
  } else {
    const compare = await github(`${repo}/compare/${last}...${head}`).catch(() => null);
    if (compare?.status === "ahead") {
      // Only the files that are part of the PR: a merge of main would bring in changes that are not its own
      const inPr = new Set(prFiles.map((f) => f.filename));
      files = (compare.files ?? []).filter((f) => inPr.has(f.filename));
      mode = `incremental, since commit ${last.slice(0, 7)}`;
    }
  }
}

files = files.filter((f) => f.patch && !SKIP.some((re) => re.test(f.filename)));

// Conversations opened by this script's reviews (the first comment starts with the severity)
async function openThreads() {
  const [owner, name] = REPOSITORY.split("/");
  const query = `query($owner: String!, $name: String!, $number: Int!, $cursor: String) {
    repository(owner: $owner, name: $name) {
      pullRequest(number: $number) {
        reviewThreads(first: 100, after: $cursor) {
          nodes {
            id isResolved path line originalLine
            comments(first: 100) { nodes { databaseId body author { login } } }
          }
          pageInfo { hasNextPage endCursor }
        }
      }
    }
  }`;
  const threads = [];
  for (let cursor = null; ; ) {
    const data = await graphql(query, { owner, name, number: Number(PR), cursor });
    const page = data.repository.pullRequest.reviewThreads;
    threads.push(...page.nodes);
    if (!page.pageInfo.hasNextPage) break;
    cursor = page.pageInfo.endCursor;
  }
  return threads.filter((t) => {
    const first = t.comments.nodes[0];
    return !t.isResolved && first && BOT.has(first.author?.login) && GEMINI_COMMENT.test(first.body);
  });
}

// The thread's last message is from a person (deleted author included): it is waiting for an answer
const answered = (t) => !BOT.has(t.comments.nodes.at(-1).author?.login);

const resolveThread = (id) =>
  graphql(
    `
      mutation ($id: ID!) {
        resolveReviewThread(input: { threadId: $id }) {
          thread {
            id
          }
        }
      }
    `,
    { id },
  );

// Already accepted (the model's last reply starts with "✅") but still open, for example after an error while
// closing: they are closed without calling the model again
const open = await openThreads();
const accepted = open.filter((t) => !answered(t) && t.comments.nodes.length > 1 && t.comments.nodes.at(-1).body.startsWith("✅"));
let resolved = 0;
for (const t of accepted) {
  try {
    await resolveThread(t.id);
    resolved++;
  } catch (error) {
    console.log(`::warning::Conversation ${t.path}: ${error.message}`);
  }
}

// Re-evaluated: conversations with a new reply and those on files touched by the new commits; all of them on a full review
const changed = new Set(files.map((f) => f.filename));
const threads = open.filter(
  (t) => !accepted.includes(t) && (answered(t) || (!threadsOnly && (mode === "full" || changed.has(t.path)))),
);

if (!files.length && !threads.length) {
  summary(
    last === head && !threadsOnly
      ? `Commit ${head.slice(0, 7)} already has a review and no conversation has new replies: nothing to do.`
      : "No changes to review and no conversations to re-evaluate.",
  );
  if (accepted.length) summary(`Already accepted conversations closed: ${resolved} of ${accepted.length}.`);
  process.exit(0);
}

// Lines of the current file around the comment, numbered as in the diff
async function excerpt(path, line, radius) {
  const ref = path.split("/").map(encodeURIComponent).join("/");
  const text = await github(`${repo}/contents/${ref}?ref=${head}`, { accept: "application/vnd.github.raw+json" }).catch(
    (error) => {
      if (error.status === 404) return null;
      throw error;
    },
  );
  if (text === null) return "(file deleted)";
  const rows = text.split("\n");
  const from = Math.max(1, line - radius);
  const to = Math.min(rows.length, line + radius);
  return rows
    .slice(from - 1, to)
    .map((row, i) => `${String(from + i).padStart(5)}  ${row}`)
    .join("\n");
}

const byKey = new Map(threads.map((t, i) => [`T${i + 1}`, t]));
let conversations = "";
for (const [key, t] of byKey) {
  const [first, ...replies] = t.comments.nodes;
  // Outdated thread (line null): the code around it has changed, the original line is only indicative
  const where = t.line ? `line ${t.line}` : `original line ${t.originalLine}, code changed since then`;
  const code = await excerpt(t.path, t.line ?? t.originalLine ?? 1, t.line ? 15 : 40);
  conversations += `## ${key} · ${t.path} (${where})
<comment>
${first.body}
</comment>
${replies.map((c) => `<reply author="${c.author?.login ?? "?"}">\n${c.body}\n</reply>`).join("\n")}
Current code:
${code}

`;
}

let diff = "";
const omitted = [];
for (const f of files) {
  const part = `### ${f.filename} (${f.status})\n${annotate(f.patch).text}\n\n`;
  if (diff.length + part.length > MAX_DIFF_CHARS) omitted.push(f.filename);
  else diff += part;
}

const prompt = `# Pull request #${PR}: ${pr.title}

${pr.body || "(no description)"}

${
  files.length
    ? `# Diff (${mode})\n${diff}${omitted.length ? `\nFiles left out because of size: ${omitted.join(", ")}\n` : ""}`
    : "# Diff\nNo new changes: only evaluate the open conversations and leave comments empty."
}
${threads.length ? `\n# Open conversations from previous reviews\n${conversations}` : ""}`;

const { result, usage } = await gemini(prompt);

const inline = [];
const loose = [];
for (const c of result.comments ?? []) {
  const lines = commentable.get(c.path);
  const severity = SEVERITIES.includes(c.severity) ? c.severity : "MEDIUM";
  const suggestion = c.suggestion ? `\n\n\`\`\`suggestion\n${c.suggestion.replace(/\n$/, "")}\n\`\`\`` : "";
  if (!lines?.has(c.line)) {
    loose.push(`- **[${severity}]** \`${c.path}:${c.line}\` — ${c.body}`);
    continue;
  }
  const multi = c.startLine && c.startLine < c.line && lines.has(c.startLine);
  inline.push({
    path: c.path,
    line: c.line,
    side: "RIGHT",
    ...(multi && { start_line: c.startLine, start_side: "RIGHT" }),
    body: `**[${severity}]** ${c.body}${suggestion}`,
  });
}

const tokens = `${MODEL} · ${usage.promptTokenCount ?? "?"} input tokens, ${usage.candidatesTokenCount ?? "?"} output, ${usage.thoughtsTokenCount ?? 0} reasoning`;
const body = (comments) =>
  [
    `## 📋 Gemini review (${mode})`,
    result.summary,
    !comments.length && !loose.length ? "No problems found." : "",
    loose.length ? `### Other remarks\n${loose.join("\n")}` : "",
    `<sub>${tokens}</sub>`,
  ]
    .filter(Boolean)
    .join("\n\n");

// Without new changes no review is published: it would mark the commit as reviewed without having read it
if (files.length) {
  const review = { commit_id: head, event: "COMMENT", body: body(inline), comments: inline };
  try {
    await github(`${repo}/pulls/${PR}/reviews`, { method: "POST", body: review });
  } catch (error) {
    // One line GitHub does not accept makes the whole review fail: fall back to comments in the review body
    if (error.status !== 422) throw error;
    console.log(`::warning::Inline comments rejected, moved into the review body: ${error.message}`);
    loose.unshift(...inline.map((c) => `- \`${c.path}:${c.line}\` — ${c.body}`));
    await github(`${repo}/pulls/${PR}/reviews`, {
      method: "POST",
      body: { commit_id: head, event: "COMMENT", body: body([]) },
    });
  }
}

// Conversations: reply and resolve if the model accepts the fix or the explanation, reply only if it does not accept
// it and the author had written; otherwise the thread stays as it is, without a message at every push
const threadState = `query($id: ID!) {
  node(id: $id) { ... on PullRequestReviewThread { isResolved comments(last: 1) { nodes { author { login } } } } }
}`;
let replied = 0;
for (const decision of result.threads ?? []) {
  const t = byKey.get(decision.id);
  if (!t) continue;
  const reply = decision.reply?.trim();
  if (!decision.resolved && !(reply && answered(t))) continue;
  try {
    // A parallel run (a push and a reply together) may have handled it already
    const now = (await graphql(threadState, { id: t.id })).node;
    if (now.isResolved || (!decision.resolved && BOT.has(now.comments.nodes[0]?.author?.login))) continue;
    await github(`${repo}/pulls/${PR}/comments/${t.comments.nodes[0].databaseId}/replies`, {
      method: "POST",
      body: { body: decision.resolved ? `✅ ${reply || "Resolved."}` : reply },
    });
    if (decision.resolved) {
      await resolveThread(t.id);
      resolved++;
    } else replied++;
  } catch (error) {
    // A thread that could not be handled must not lose the others or the review already published
    console.log(`::warning::Conversation ${t.path}: ${error.message}`);
  }
}

summary(
  [
    files.length
      ? `Review ${mode}: ${files.length} files, ${inline.length} inline comments, ${loose.length} in the body.`
      : "No new review: conversations only.",
    accepted.length ? `Already accepted conversations to close: ${accepted.length}.` : "",
    threads.length || accepted.length
      ? `Conversations re-evaluated: ${threads.length}, resolved ${resolved}, replied and still open ${replied}.`
      : "",
    tokens,
  ]
    .filter(Boolean)
    .join("\n\n"),
);
