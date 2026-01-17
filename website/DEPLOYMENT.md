# Deployment Guide for localrouter.ai

This guide explains how to deploy the LocalRouter website to https://localrouter.ai using GitHub Pages with a custom domain.

## Prerequisites

- Access to the GitHub repository: LocalRouter/LocalRouter
- Access to DNS management for localrouter.ai domain
- GitHub Actions enabled on the repository

## Step 1: Configure GitHub Pages

1. Go to the repository on GitHub
2. Navigate to **Settings** > **Pages**
3. Under "Build and deployment":
   - Source: Select **GitHub Actions**
   - This allows the workflow to deploy automatically

## Step 2: Configure DNS Records

At your domain registrar (where you manage localrouter.ai), set up the following DNS records:

### Option A: Apex Domain (recommended)

Add these `A` records for the apex domain:

```
Type    Name    Value
A       @       185.199.108.153
A       @       185.199.109.153
A       @       185.199.110.153
A       @       185.199.111.153
```

### Option B: WWW Subdomain

If you want to use www.localrouter.ai:

```
Type     Name    Value
CNAME    www     LocalRouter.github.io
```

### Add Both (Best Practice)

For best compatibility, add both the A records and a CNAME record:

```
Type     Name    Value
A        @       185.199.108.153
A        @       185.199.109.153
A        @       185.199.110.153
A        @       185.199.111.153
CNAME    www     LocalRouter.github.io
```

## Step 3: Verify Custom Domain in GitHub

1. Go to **Settings** > **Pages** in your repository
2. Under "Custom domain", you should see `localrouter.ai`
3. Click "Save" if needed
4. Wait for the DNS check to complete (this can take a few minutes to 48 hours)
5. Once verified, check "Enforce HTTPS"

## Step 4: Deploy

The deployment happens automatically via GitHub Actions:

1. Make changes to files in the `website/` directory
2. Commit and push to the `master` branch
3. The workflow `.github/workflows/deploy-website.yml` will:
   - Build the React app
   - Add the CNAME file
   - Deploy to GitHub Pages
4. Your changes will be live at https://localrouter.ai

### Manual Deployment

You can also trigger a manual deployment:

1. Go to **Actions** tab in GitHub
2. Select "Deploy Website to GitHub Pages" workflow
3. Click "Run workflow"
4. Select the `master` branch
5. Click "Run workflow"

## Step 5: Verify Deployment

Once the workflow completes:

1. Visit https://localrouter.ai
2. Check that the site loads correctly
3. Verify HTTPS is working (look for the padlock icon)
4. Test navigation between pages

## Troubleshooting

### DNS Not Propagating

- DNS changes can take up to 48 hours to propagate globally
- Use https://dnschecker.org to check propagation status
- Clear your browser cache and DNS cache

### HTTPS Not Available

- Wait for DNS to fully propagate
- Make sure "Enforce HTTPS" is enabled in GitHub Pages settings
- GitHub may take a few minutes to provision the SSL certificate

### Deployment Failing

- Check the Actions tab for error logs
- Ensure all dependencies are correctly installed
- Verify the workflow has proper permissions

### Custom Domain Not Working

- Verify CNAME file exists in the `website/public/` directory
- Check that DNS records are correct
- Ensure the custom domain is saved in GitHub Pages settings

## Workflow Details

The deployment workflow (`.github/workflows/deploy-website.yml`) runs when:

- Code is pushed to the `master` branch in the `website/` directory
- The workflow is manually triggered via GitHub Actions
- Changes are made to the workflow file itself

### Build Process

1. Checkout code
2. Setup Node.js 18
3. Install dependencies
4. Build the React app with Vite
5. Add CNAME file to dist folder
6. Upload artifact to GitHub Pages
7. Deploy to production

## Monitoring

- GitHub Actions provides logs for each deployment
- Check the Actions tab to see deployment history
- Each deployment takes approximately 1-2 minutes

## Rollback

To rollback to a previous version:

1. Go to the Actions tab
2. Find the last successful deployment
3. Click "Re-run all jobs"

Or revert the commit and push:

```bash
git revert <commit-hash>
git push origin master
```

## Additional Resources

- [GitHub Pages Documentation](https://docs.github.com/en/pages)
- [Configuring a custom domain](https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site)
- [GitHub Actions Documentation](https://docs.github.com/en/actions)

## Support

For issues with deployment:
- Check GitHub Actions logs
- Review DNS settings
- Contact GitHub Support for Pages-specific issues
