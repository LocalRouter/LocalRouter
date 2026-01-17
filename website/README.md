# LocalRouter Website

This is the marketing website for LocalRouter, built with React, TypeScript, Vite, and Tailwind CSS.

## Development

Install dependencies:

```bash
npm install
```

Run the development server:

```bash
npm run dev
```

The website will be available at `http://localhost:5173`.

## Building for Production

Build the website:

```bash
npm run build
```

The built files will be in the `dist/` directory, ready to be deployed to any static hosting service.

Preview the production build:

```bash
npm run preview
```

## Deployment

This website is configured to deploy to **localrouter.ai** via GitHub Pages.

### GitHub Pages Deployment (with Custom Domain)

The website is already set up with a GitHub Actions workflow at `.github/workflows/deploy-website.yml`. To deploy:

1. **Configure GitHub Pages**:
   - Go to repository Settings > Pages
   - Set Source to "GitHub Actions"
   - The custom domain `localrouter.ai` is configured via CNAME

2. **Set up DNS** (at your domain registrar):
   - Add an `A` record pointing to GitHub Pages:
     ```
     A    @    185.199.108.153
     A    @    185.199.109.153
     A    @    185.199.110.153
     A    @    185.199.111.153
     ```
   - Or use a `CNAME` record (for subdomain):
     ```
     CNAME    www    LocalRouter.github.io
     ```

3. **Enable HTTPS**:
   - Go to repository Settings > Pages
   - Check "Enforce HTTPS" (after DNS propagates)

4. **Deploy**:
   - Push changes to the `master` branch
   - The workflow automatically builds and deploys the site
   - Site will be live at https://localrouter.ai

### Alternative Deployment Options

- **Netlify**: Connect your repository and deploy automatically
- **Vercel**: Import the repository and deploy
- **Any static hosting**: Upload the `dist/` folder

## Structure

```
website/
├── src/
│   ├── pages/
│   │   ├── Home.tsx          # Home page with hero and features
│   │   └── Download.tsx      # Download page with OS installers
│   ├── components/
│   │   ├── Navigation.tsx    # Top navigation bar
│   │   └── Footer.tsx        # Footer component
│   ├── App.tsx               # Main app component with routing
│   ├── main.tsx              # Entry point
│   └── index.css             # Global styles with Tailwind
├── public/                   # Static assets
├── index.html                # HTML template
├── vite.config.ts            # Vite configuration
├── tailwind.config.js        # Tailwind CSS configuration
└── package.json              # Dependencies and scripts
```

## Technologies

- **React 18**: UI framework
- **TypeScript**: Type safety
- **Vite**: Build tool and dev server
- **Tailwind CSS**: Utility-first CSS framework
- **React Router**: Client-side routing

## License

MIT
