import { renderToString } from 'react-dom/server'
import { StaticRouter } from 'react-router-dom'
import { AppLayout } from './App'
export { sitePages, siteOrigin, notFoundPage } from './site-pages'

export function render(path: string) {
  return renderToString(<StaticRouter location={path}><AppLayout /></StaticRouter>)
}
