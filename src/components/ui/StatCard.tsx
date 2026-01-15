interface StatCardProps {
  title: string
  value: string | number
}

export default function StatCard({ title, value }: StatCardProps) {
  return (
    <div className="bg-gradient-to-r from-indigo-500 to-purple-600 text-white p-6 rounded-lg shadow-lg">
      <h3 className="text-sm opacity-90 mb-2">{title}</h3>
      <div className="text-3xl font-bold">{value}</div>
    </div>
  )
}
