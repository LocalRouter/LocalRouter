export default function Footer() {
  return (
    <footer className="bg-gray-900 text-gray-300">
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-12">
        <div className="grid grid-cols-1 md:grid-cols-3 gap-8">
          <div>
            <div className="flex items-center space-x-2 mb-4">
              <div className="w-8 h-8 bg-gradient-to-br from-blue-500 to-purple-600 rounded-lg"></div>
              <span className="text-xl font-bold text-white">LocalRouter</span>
            </div>
            <p className="text-gray-400">
              Intelligent AI model routing with OpenAI-compatible API
            </p>
          </div>

          <div>
            <h3 className="text-white font-semibold mb-4">Resources</h3>
            <ul className="space-y-2">
              <li>
                <a href="https://github.com/LocalRouter/LocalRouter" target="_blank" rel="noopener noreferrer" className="hover:text-white transition">
                  GitHub
                </a>
              </li>
              <li>
                <a href="https://github.com/LocalRouter/LocalRouter/blob/master/README.md" target="_blank" rel="noopener noreferrer" className="hover:text-white transition">
                  Documentation
                </a>
              </li>
              <li>
                <a href="https://github.com/LocalRouter/LocalRouter/issues" target="_blank" rel="noopener noreferrer" className="hover:text-white transition">
                  Issues
                </a>
              </li>
            </ul>
          </div>

          <div>
            <h3 className="text-white font-semibold mb-4">Legal</h3>
            <ul className="space-y-2">
              <li>
                <a href="https://github.com/LocalRouter/LocalRouter/blob/master/LICENSE" target="_blank" rel="noopener noreferrer" className="hover:text-white transition">
                  License
                </a>
              </li>
            </ul>
          </div>
        </div>

        <div className="border-t border-gray-800 mt-8 pt-8 text-center text-gray-400">
          <p>&copy; {new Date().getFullYear()} LocalRouter. All rights reserved.</p>
        </div>
      </div>
    </footer>
  )
}
