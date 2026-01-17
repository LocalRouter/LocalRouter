export default function Download() {
  return (
    <div className="bg-white">
      {/* Hero Section */}
      <div className="bg-gradient-to-b from-gray-50 to-white py-16">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
          <div className="text-center">
            <h1 className="text-5xl font-bold text-gray-900 mb-6">
              Download LocalRouter
            </h1>
            <p className="text-xl text-gray-600 max-w-3xl mx-auto">
              Get started with LocalRouter on your platform. Available for macOS, Windows, and Linux.
            </p>
          </div>
        </div>
      </div>

      {/* Download Options */}
      <div className="py-16">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
          <div className="grid grid-cols-1 md:grid-cols-3 gap-8">
            {/* macOS */}
            <div className="bg-white border-2 border-gray-200 rounded-xl p-8 hover:border-blue-500 hover:shadow-lg transition">
              <div className="flex justify-center mb-6">
                <svg className="w-20 h-20 text-gray-700" fill="currentColor" viewBox="0 0 24 24">
                  <path d="M18.71 19.5c-.83 1.24-1.71 2.45-3.05 2.47-1.34.03-1.77-.79-3.29-.79-1.53 0-2 .77-3.27.82-1.31.05-2.3-1.32-3.14-2.53C4.25 17 2.94 12.45 4.7 9.39c.87-1.52 2.43-2.48 4.12-2.51 1.28-.02 2.5.87 3.29.87.78 0 2.26-1.07 3.81-.91.65.03 2.47.26 3.64 1.98-.09.06-2.17 1.28-2.15 3.81.03 3.02 2.65 4.03 2.68 4.04-.03.07-.42 1.44-1.38 2.83M13 3.5c.73-.83 1.94-1.46 2.94-1.5.13 1.17-.34 2.35-1.04 3.19-.69.85-1.83 1.51-2.95 1.42-.15-1.15.41-2.35 1.05-3.11z"/>
                </svg>
              </div>
              <h3 className="text-2xl font-bold text-gray-900 mb-4 text-center">macOS</h3>
              <ul className="space-y-2 mb-8 text-gray-600">
                <li className="flex items-center">
                  <svg className="w-5 h-5 text-green-500 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  macOS 11+ (Big Sur and later)
                </li>
                <li className="flex items-center">
                  <svg className="w-5 h-5 text-green-500 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  Intel and Apple Silicon
                </li>
                <li className="flex items-center">
                  <svg className="w-5 h-5 text-green-500 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  DMG installer
                </li>
              </ul>
              <a
                href="https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_macOS.dmg"
                className="block w-full bg-gradient-to-r from-blue-500 to-purple-600 text-white text-center px-6 py-3 rounded-lg font-semibold hover:from-blue-600 hover:to-purple-700 transition"
              >
                Download for macOS
              </a>
              <div className="mt-4 text-center">
                <a
                  href="https://github.com/LocalRouter/LocalRouter/releases/latest"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-sm text-blue-600 hover:text-blue-700"
                >
                  View all releases
                </a>
              </div>
            </div>

            {/* Windows */}
            <div className="bg-white border-2 border-gray-200 rounded-xl p-8 hover:border-blue-500 hover:shadow-lg transition">
              <div className="flex justify-center mb-6">
                <svg className="w-20 h-20 text-gray-700" fill="currentColor" viewBox="0 0 24 24">
                  <path d="M0 3.449L9.75 2.1v9.451H0m10.949-9.602L24 0v11.4H10.949M0 12.6h9.75v9.451L0 20.699M10.949 12.6H24V24l-12.9-1.801"/>
                </svg>
              </div>
              <h3 className="text-2xl font-bold text-gray-900 mb-4 text-center">Windows</h3>
              <ul className="space-y-2 mb-8 text-gray-600">
                <li className="flex items-center">
                  <svg className="w-5 h-5 text-green-500 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  Windows 10+ (64-bit)
                </li>
                <li className="flex items-center">
                  <svg className="w-5 h-5 text-green-500 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  MSI installer
                </li>
                <li className="flex items-center">
                  <svg className="w-5 h-5 text-green-500 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  Portable EXE available
                </li>
              </ul>
              <a
                href="https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_Windows.msi"
                className="block w-full bg-gradient-to-r from-blue-500 to-purple-600 text-white text-center px-6 py-3 rounded-lg font-semibold hover:from-blue-600 hover:to-purple-700 transition"
              >
                Download for Windows
              </a>
              <div className="mt-4 text-center">
                <a
                  href="https://github.com/LocalRouter/LocalRouter/releases/latest"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-sm text-blue-600 hover:text-blue-700"
                >
                  View all releases
                </a>
              </div>
            </div>

            {/* Linux */}
            <div className="bg-white border-2 border-gray-200 rounded-xl p-8 hover:border-blue-500 hover:shadow-lg transition">
              <div className="flex justify-center mb-6">
                <svg className="w-20 h-20 text-gray-700" fill="currentColor" viewBox="0 0 24 24">
                  <path d="M12.504 0c-.155 0-.315.008-.48.021-4.226.333-3.105 4.807-3.17 6.298-.076 1.092-.3 1.816-1.049 3.08-.232.392-.41.858-.552 1.402-.276 1.06-.43 2.569-.43 4.298 0 1.729.154 3.238.43 4.298.142.544.32 1.01.552 1.402.748 1.264.973 1.988 1.049 3.08.065 1.491-.944 5.965 3.17 6.298.165.013.325.021.48.021s.315-.008.48-.021c4.114-.333 3.105-4.807 3.17-6.298.076-1.092.3-1.816 1.049-3.08.232-.392.41-.858.552-1.402.276-1.06.43-2.569.43-4.298 0-1.729-.154-3.238-.43-4.298-.142-.544-.32-1.01-.552-1.402-.749-1.264-.973-1.988-1.049-3.08-.065-1.491.944-5.965-3.17-6.298-.165-.013-.325-.021-.48-.021zm-.002 2.583c.085 0 .167.005.248.014 1.017.113.904 1.535.904 2.083 0 1.168-.271 2.245-.679 3.067-.204.411-.44.793-.705 1.147-.53.708-.674 1.002-.674 2.106 0 1.104.144 1.398.674 2.106.265.354.501.736.705 1.147.408.822.679 1.899.679 3.067 0 .548.113 1.97-.904 2.083-.081.009-.163.014-.248.014s-.167-.005-.248-.014c-1.017-.113-.904-1.535-.904-2.083 0-1.168.271-2.245.679-3.067.204-.411.44-.793.705-1.147.53-.708.674-1.002.674-2.106 0-1.104-.144-1.398-.674-2.106-.265-.354-.501-.736-.705-1.147-.408-.822-.679-1.899-.679-3.067 0-.548-.113-1.97.904-2.083.081-.009.163-.014.248-.014z"/>
                </svg>
              </div>
              <h3 className="text-2xl font-bold text-gray-900 mb-4 text-center">Linux</h3>
              <ul className="space-y-2 mb-8 text-gray-600">
                <li className="flex items-center">
                  <svg className="w-5 h-5 text-green-500 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  Ubuntu, Debian (DEB)
                </li>
                <li className="flex items-center">
                  <svg className="w-5 h-5 text-green-500 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  Fedora, RHEL (RPM)
                </li>
                <li className="flex items-center">
                  <svg className="w-5 h-5 text-green-500 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  AppImage (Universal)
                </li>
              </ul>
              <div className="space-y-3">
                <a
                  href="https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_Linux.deb"
                  className="block w-full bg-gradient-to-r from-blue-500 to-purple-600 text-white text-center px-6 py-3 rounded-lg font-semibold hover:from-blue-600 hover:to-purple-700 transition"
                >
                  Download DEB
                </a>
                <a
                  href="https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_Linux.rpm"
                  className="block w-full bg-gray-700 text-white text-center px-6 py-3 rounded-lg font-semibold hover:bg-gray-800 transition"
                >
                  Download RPM
                </a>
              </div>
              <div className="mt-4 text-center">
                <a
                  href="https://github.com/LocalRouter/LocalRouter/releases/latest"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-sm text-blue-600 hover:text-blue-700"
                >
                  View all releases
                </a>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Installation Instructions */}
      <div className="bg-gray-50 py-16">
        <div className="max-w-4xl mx-auto px-4 sm:px-6 lg:px-8">
          <h2 className="text-3xl font-bold text-gray-900 mb-8 text-center">
            Installation & Setup
          </h2>

          <div className="space-y-6">
            <div className="bg-white rounded-lg p-6 shadow-sm">
              <h3 className="text-xl font-semibold text-gray-900 mb-3 flex items-center">
                <span className="bg-blue-500 text-white w-8 h-8 rounded-full flex items-center justify-center mr-3">1</span>
                Download the installer
              </h3>
              <p className="text-gray-600 ml-11">
                Choose the appropriate installer for your operating system from the options above.
              </p>
            </div>

            <div className="bg-white rounded-lg p-6 shadow-sm">
              <h3 className="text-xl font-semibold text-gray-900 mb-3 flex items-center">
                <span className="bg-blue-500 text-white w-8 h-8 rounded-full flex items-center justify-center mr-3">2</span>
                Install LocalRouter
              </h3>
              <p className="text-gray-600 ml-11">
                Run the installer and follow the on-screen instructions. LocalRouter will be installed and ready to use.
              </p>
            </div>

            <div className="bg-white rounded-lg p-6 shadow-sm">
              <h3 className="text-xl font-semibold text-gray-900 mb-3 flex items-center">
                <span className="bg-blue-500 text-white w-8 h-8 rounded-full flex items-center justify-center mr-3">3</span>
                Configure providers
              </h3>
              <p className="text-gray-600 ml-11">
                Open LocalRouter and configure your AI providers. Add API keys for OpenAI, Anthropic, or connect to local Ollama.
              </p>
            </div>

            <div className="bg-white rounded-lg p-6 shadow-sm">
              <h3 className="text-xl font-semibold text-gray-900 mb-3 flex items-center">
                <span className="bg-blue-500 text-white w-8 h-8 rounded-full flex items-center justify-center mr-3">4</span>
                Start using the API
              </h3>
              <p className="text-gray-600 ml-11 mb-4">
                LocalRouter starts a local server at <code className="bg-gray-100 px-2 py-1 rounded">http://localhost:3000</code>.
                Use it as a drop-in replacement for the OpenAI API.
              </p>
              <div className="ml-11 bg-gray-900 text-gray-100 p-4 rounded-lg font-mono text-sm overflow-x-auto">
                <pre>{`curl http://localhost:3000/v1/chat/completions \\
  -H "Content-Type: application/json" \\
  -H "Authorization: Bearer lr-your-api-key" \\
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'`}</pre>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* System Requirements */}
      <div className="py-16">
        <div className="max-w-4xl mx-auto px-4 sm:px-6 lg:px-8">
          <h2 className="text-3xl font-bold text-gray-900 mb-8 text-center">
            System Requirements
          </h2>
          <div className="bg-white rounded-lg shadow-sm p-8">
            <div className="grid grid-cols-1 md:grid-cols-3 gap-8">
              <div>
                <h3 className="font-semibold text-gray-900 mb-3">macOS</h3>
                <ul className="text-gray-600 space-y-2 text-sm">
                  <li>• macOS 11 (Big Sur) or later</li>
                  <li>• 4GB RAM minimum</li>
                  <li>• 200MB disk space</li>
                </ul>
              </div>
              <div>
                <h3 className="font-semibold text-gray-900 mb-3">Windows</h3>
                <ul className="text-gray-600 space-y-2 text-sm">
                  <li>• Windows 10 or later (64-bit)</li>
                  <li>• 4GB RAM minimum</li>
                  <li>• 200MB disk space</li>
                </ul>
              </div>
              <div>
                <h3 className="font-semibold text-gray-900 mb-3">Linux</h3>
                <ul className="text-gray-600 space-y-2 text-sm">
                  <li>• Modern Linux distribution</li>
                  <li>• 4GB RAM minimum</li>
                  <li>• 200MB disk space</li>
                </ul>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Help Section */}
      <div className="bg-gradient-to-b from-white to-gray-50 py-16">
        <div className="max-w-4xl mx-auto px-4 sm:px-6 lg:px-8 text-center">
          <h2 className="text-3xl font-bold text-gray-900 mb-6">
            Need Help?
          </h2>
          <p className="text-xl text-gray-600 mb-8">
            Check out our documentation or join the community for support.
          </p>
          <div className="flex justify-center space-x-4">
            <a
              href="https://github.com/LocalRouter/LocalRouter/blob/master/README.md"
              target="_blank"
              rel="noopener noreferrer"
              className="bg-gray-100 text-gray-900 px-6 py-3 rounded-lg font-semibold hover:bg-gray-200 transition"
            >
              View Documentation
            </a>
            <a
              href="https://github.com/LocalRouter/LocalRouter/issues"
              target="_blank"
              rel="noopener noreferrer"
              className="bg-gray-100 text-gray-900 px-6 py-3 rounded-lg font-semibold hover:bg-gray-200 transition"
            >
              Report an Issue
            </a>
          </div>
        </div>
      </div>
    </div>
  )
}
