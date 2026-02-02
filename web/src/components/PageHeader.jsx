function PageHeader({ title, icon: Icon, children }) {
  return (
    <div className="bg-gray-800 border-b border-gray-700 px-6 py-4">
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-semibold flex items-center gap-3">
          {Icon && <Icon className="w-5 h-5 text-blue-400" />}
          {title}
        </h1>
        {children && <div className="flex items-center gap-2">{children}</div>}
      </div>
    </div>
  );
}

export default PageHeader;
