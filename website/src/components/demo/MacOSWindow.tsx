interface MacOSWindowProps {
  title: string
  children: React.ReactNode
  width?: number
  height?: number
}

export function MacOSWindow({
  title,
  children,
  width = 1000,
  height = 600,
}: MacOSWindowProps) {
  return (
    <div
      className="rounded-lg overflow-hidden shadow-2xl border border-gray-300/50"
      style={{ width, maxWidth: '100%' }}
    >
      {/* Title bar */}
      <div className="h-7 bg-gradient-to-b from-[#e8e8e8] to-[#d3d3d3] flex items-center px-3 border-b border-gray-400/30">
        <div className="flex gap-2">
          <span className="w-3 h-3 rounded-full bg-[#ff5f57] border border-[#e14640]" />
          <span className="w-3 h-3 rounded-full bg-[#febc2e] border border-[#d4a029]" />
          <span className="w-3 h-3 rounded-full bg-[#28c840] border border-[#24a732]" />
        </div>
        <span className="flex-1 text-center text-[13px] font-medium text-gray-600">
          {title}
        </span>
        <div className="w-14" /> {/* Spacer to balance traffic lights */}
      </div>

      {/* Content */}
      <div className="bg-white overflow-hidden" style={{ height }}>
        {children}
      </div>
    </div>
  )
}
