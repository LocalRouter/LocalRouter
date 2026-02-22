import { BrowserRouter as Router, Routes, Route, Navigate } from 'react-router-dom'
import Navigation from './components/Navigation'
import Footer from './components/Footer'
import Home from './pages/Home'
import Download from './pages/Download'
import Demo from './pages/Demo'
import Docs from './pages/Docs'

function App() {
  return (
    <Router>
      <Routes>
        {/* Demo route - full screen, no navigation */}
        <Route path="/demo" element={<Demo />} />

        {/* Main site routes */}
        <Route path="*" element={
          <div className="min-h-screen flex flex-col">
            <Navigation />
            <main className="flex-grow">
              <Routes>
                <Route path="/" element={<Home />} />
                <Route path="/docs" element={<Navigate to="/docs/introduction" replace />} />
                <Route path="/docs/:sectionId" element={<Docs />} />
                <Route path="/download" element={<Download />} />
              </Routes>
            </main>
            <Footer />
          </div>
        } />
      </Routes>
    </Router>
  )
}

export default App
