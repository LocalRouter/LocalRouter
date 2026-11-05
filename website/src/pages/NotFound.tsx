import { Link } from 'react-router-dom'

export default function NotFound() {
  return (
    <section className="mx-auto max-w-3xl px-6 py-24 text-center">
      <h1 className="text-4xl font-bold">Page not found</h1>
      <p className="mt-6 text-muted-foreground">This page does not exist. Explore LocalRouter below.</p>
      <div className="mt-8 flex justify-center gap-6 text-primary">
        <Link to="/">Home</Link>
        <Link to="/docs/introduction">Documentation</Link>
        <Link to="/research">Research</Link>
      </div>
    </section>
  )
}
