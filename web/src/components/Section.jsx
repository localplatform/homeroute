function Section({ title, children, contrast = false, className = '' }) {
  return (
    <div className={`border-b border-gray-700 ${contrast ? 'bg-gray-800/50' : 'bg-gray-900'} ${className}`}>
      {title && (
        <div className="px-6 py-3 border-b border-gray-700/50">
          <h2 className="text-sm font-semibold text-gray-400 uppercase tracking-wider">{title}</h2>
        </div>
      )}
      <div className="px-6 py-4">
        {children}
      </div>
    </div>
  );
}

export default Section;
