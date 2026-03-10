import { BrowserRouter as Router, Routes, Route, Navigate, useLocation } from 'react-router-dom'
import Navigation from './components/Navigation'
import Footer from './components/Footer'
import Home from './pages/Home'
import Download from './pages/Download'
import Demo from './pages/Demo'
import Docs from './pages/Docs'
import Research from './pages/Research'

function AppLayout() {
  const { pathname } = useLocation()
  const hasSidebar = pathname.startsWith('/docs') || pathname.startsWith('/research')

  return (
    <div className="h-screen flex flex-col">
      <Navigation />
      {hasSidebar ? (
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
      <Routes>
        {/* Demo route - full screen, no navigation */}
        <Route path="/demo" element={<Demo />} />

        {/* Main site routes */}
        <Route path="*" element={<AppLayout />} />
      </Routes>
    </Router>
  )
}

export default App
