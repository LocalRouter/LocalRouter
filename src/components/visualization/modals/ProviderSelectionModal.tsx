import Modal from '../../ui/Modal';

interface ProviderSelectionModalProps {
  isOpen: boolean;
  onClose: () => void;
  onCreate?: () => void;
}

export function ProviderSelectionModal({
  isOpen,
  onClose,
}: ProviderSelectionModalProps) {
  return (
    <Modal isOpen={isOpen} onClose={onClose} title="Add Provider">
      <div className="space-y-4">
        <div className="bg-blue-50 border border-blue-200 rounded-lg p-4">
          <p className="text-blue-900 text-sm">
            To add a new provider, please use the <strong>Providers</strong> tab which provides
            the full provider configuration interface.
          </p>
        </div>

        <div className="flex justify-end">
          <button
            onClick={onClose}
            className="px-4 py-2 bg-gray-200 text-gray-700 rounded-lg hover:bg-gray-300 transition-colors"
          >
            Close
          </button>
        </div>
      </div>
    </Modal>
  );
}
