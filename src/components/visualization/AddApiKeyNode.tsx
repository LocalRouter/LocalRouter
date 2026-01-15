interface AddApiKeyNodeData {
  nodeType: 'AddNode';
  onClick?: () => void;
}

export function AddApiKeyNode({ data }: { data: AddApiKeyNodeData }) {
  return (
    <div
      className="px-6 py-4 rounded-lg border-2 border-dashed border-gray-400 bg-gray-50 shadow-md min-w-[180px] cursor-pointer hover:bg-gray-100 hover:border-gray-500 transition-all"
      onClick={data.onClick}
    >
      <div className="flex flex-col items-center justify-center gap-2">
        <div className="text-3xl text-gray-500">+</div>
        <div className="text-sm font-semibold text-gray-700">Add API Key</div>
      </div>
    </div>
  );
}
