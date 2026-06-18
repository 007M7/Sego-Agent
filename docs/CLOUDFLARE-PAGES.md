# Deploy Sego Launch Site to Cloudflare Pages

This guide deploys the Sego launch landing page from `docs/index.html`.

## Recommended setup

- Platform: Cloudflare Pages
- Source: GitHub repository `007M7/Sego-Agent`
- Project name: `sego-agent`
- Production branch: `main`
- Build command: leave empty
- Build output directory: `docs`

Cloudflare Pages will serve:

```text
docs/index.html
```

as the site homepage.

## Steps

1. Open Cloudflare Pages:

   https://pages.cloudflare.com/

2. Click **Create a project**.

3. Connect the GitHub repository:

   ```text
   007M7/Sego-Agent
   ```

4. Use these build settings:

   ```text
   Framework preset: None
   Build command: empty
   Build output directory: docs
   Root directory: /
   Production branch: main
   ```

5. Deploy.

6. After deployment, Cloudflare will give a URL like:

   ```text
   https://sego-agent.pages.dev
   ```

7. Open the URL and verify:

   - Download Sego button opens GitHub Releases
   - Apply for Free Sego Audit opens the free audit GitHub issue template
   - Request Private Audit opens the private audit GitHub issue template
   - View GitHub opens the repository

## Temporary form strategy

The launch site currently uses GitHub Issue templates instead of Tally/Fillout.

Reason:

- Tally creation was blocked during Day 1 launch setup
- GitHub Issues are already available
- This keeps the launch moving without waiting for a form vendor

When Tally or Fillout is ready, replace only the CTA links in `docs/index.html`.

## Public issue privacy warning

Because GitHub Issues are public, users should not include:

- production credentials
- API keys
- private customer data
- private source code
- unreleased business details

The issue templates and landing page both include this warning.

## Later upgrade path

After 20+ leads or 3+ paid-intent users:

1. Replace GitHub Issues with Tally/Fillout or a custom form
2. Add Cloudflare Web Analytics
3. Add a private audit intake flow
4. Build a small backend for audit requests
5. Evaluate GitHub App / PR bot
