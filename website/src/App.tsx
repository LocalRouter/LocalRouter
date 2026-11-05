import { BrowserRouter as Router, Routes, Route, Navigate, useLocation } from 'react-router-dom'
import { lazy, Suspense } from 'react'
import PageMetadata from './components/PageMetadata'
import { getSitePage } from './site-pages'
import NotFound from './pages/NotFound'
import Navigation from './components/Navigation'
import Footer from './components/Footer'
import Home from './pages/Home'
import Download from './pages/Download'
import Docs from './pages/Docs'
import Research from './pages/Research'

const Demo = lazy(() => import('./pages/Demo'))
const SecretRedactionPreview = lazy(() => import('./pages/SecretRedactionPreview'))

export function AppLayout() {
  const { pathname } = useLocation()
  const hasSidebar = pathname.startsWith('/docs') || pathname.startsWith('/research')

  const isKnownPage = getSitePage(pathname) !== undefined

  return (
    <div className="h-screen flex flex-col">
      <Navigation />
      {!isKnownPage ? (
        <><main className="flex-grow"><NotFound /></main><Footer /></>
      ) : hasSidebar ? (
        <Routes>
          <Route path="/docs" element={<Navigate to="/docs/introduction" replace />} />
          <Route path="/docs/:sectionId" element={<Docs />} />
          <Route path="/research" element={<Research />} />
          <Route path="/research/:paperId" element={<Research />} />
        </Routes>
      ) : (
        <>
          <main className="flex-grow">
            <Routes>
              <Route path="/" element={<Home />} />
              <Route path="/download" element={<Download />} />
            </Routes>
          </main>
          <Footer />
        </>
      )}
    </div>
  )
}

function App() {
  return (
    <Router>
      <PageMetadata />
      <Suspense fallback={<div className="p-8">Loading LocalRouter…</div>}>
        <Routes>
          {import.meta.env.DEV && <Route path="/__preview/secret-redaction" element={<SecretRedactionPreview />} />}
          {/* Demo route - full screen, no navigation */}
          <Route path="/demo" element={<Demo />} />

          {/* Main site routes */}
          <Route path="*" element={<AppLayout />} />
        </Routes>
      </Suspense>
    </Router>
  )
}

export default App
