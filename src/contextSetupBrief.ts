// Portable setup brief copied from Settings -> Sources -> Context ("Set up without Hollow").
// Kept self-contained and generic so any user can bootstrap the four context layers with any AI assistant.

export const SETUP_BRIEF = `# Context Layer Setup Brief

Follow this brief to set up the four context layers (home, personal, org, and project) on this computer.

## Layers

1. **Home** (\`home/<machine-slug>.md\` in your personal repo): machine-specific paths, build limits, and local setup.
2. **Personal** (\`personal-context\` private repo): preferences, memory, and learnings that follow you across machines.
3. **Organization** (\`org-context\` read-only repo, optional): shared team conventions and reference docs.
4. **Project** (\`AGENTS.md\` or \`CLAUDE.md\` inside each repository): repo-specific architecture and workflow rules.

## Steps

1. Create a private personal context repository if you do not have one yet:
   \`\`\`bash
   gh repo create personal-context --private --add-readme
   \`\`\`
2. Clone it locally (for example, \`~/.context/personal\` or \`~/Hollow/Context/personal\`).
3. Create \`INDEX.md\` and \`home/<machine-slug>.md\` using the templates below.
4. Never commit secrets, tokens, or private credentials into any layer.

## Appendix: templates

### \`INDEX.md\`

\`\`\`markdown
# Personal Context

- Machine guide: \`home/<machine-slug>.md\`
- Working style and preferences: \`me/preferences.md\`
\`\`\`

### \`home/<machine-slug>.md\`

\`\`\`markdown
# Machine Profile

- OS and architecture
- Workspace root path
- Local build and test concurrency caps
\`\`\`
`;
