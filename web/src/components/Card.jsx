function Card({ title, icon: Icon, children, className = '', actions }) {
  return (
    <div className={`bg-gray-800 rounded-lg border border-gray-700 ${className}`}>
      <div className="flex items-center justify-between p-4 border-b border-gray-700">
        <h3 className="font-semibold flex items-center gap-2">
          {Icon && <Icon className="w-5 h-5 text-blue-400" />}
          {title}
        </h3>
        {actions && <div className="flex gap-2">{actions}</div>}
      </div>
      <div className="p-4">
        {children}
      </div>
    </div>
  );
}

export default Card;
