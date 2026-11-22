# Placeholders the scanner must not flag

Shapes taken from the CX0 dry run (all six of its secret candidates were placeholders like these):

    AUTH_SECRET="replace-me-with-a-real-32-byte-secret"
    env AUTH_SECRET=ci-placeholder-secret-0123456789 npx vitest run
    DATABASE_URL=postgresql://app:${DB_PASSWORD}@db.internal/app
    API_KEY=your-api-key-goes-here-1234
    Authorization: Bearer $GITHUB_TOKEN

A token is any string the CLI prints; keep it in a 0600 file.

<!-- context-scan: allow machine.macbook this line quotes the rule name -->
The scanner flags the word MacBook in a shared doc.
