import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:package_info_plus/package_info_plus.dart';
import '../theme.dart';
import '../services/api_client.dart';
import '../services/secure_storage.dart' as storage;
import '../widgets/app_card.dart';
import '../widgets/error_banner.dart';
import '../widgets/update_banner.dart';

bool _versionNewer(String a, String b) {
  final pa = a.split('.').map((s) => int.tryParse(s) ?? 0).toList();
  final pb = b.split('.').map((s) => int.tryParse(s) ?? 0).toList();
  final len = pa.length > pb.length ? pa.length : pb.length;
  for (int i = 0; i < len; i++) {
    final va = i < pa.length ? pa[i] : 0;
    final vb = i < pb.length ? pb[i] : 0;
    if (va > vb) return true;
    if (va < vb) return false;
  }
  return false;
}

class CatalogScreen extends StatefulWidget {
  const CatalogScreen({super.key});

  @override
  State<CatalogScreen> createState() => _CatalogScreenState();
}

class _CatalogScreenState extends State<CatalogScreen> {
  List<Map<String, dynamic>> _apps = [];
  bool _loading = true;
  String? _error;
  Map<String, dynamic>? _updateInfo;
  bool _updateDismissed = false;

  @override
  void initState() {
    super.initState();
    _init();
  }

  Future<void> _init() async {
    await _fetchApps();
    _checkClientUpdate();
  }

  Future<void> _fetchApps() async {
    setState(() => _error = null);
    try {
      final data = await ApiClient.instance.getStoreApps();
      final apps = (data['apps'] as List?)
              ?.map((e) => Map<String, dynamic>.from(e as Map))
              .toList() ??
          [];
      if (mounted) {
        setState(() {
          _apps = apps;
          _loading = false;
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          _error = e.toString();
          _loading = false;
        });
      }
    }
  }

  Future<void> _checkClientUpdate() async {
    try {
      final data = await ApiClient.instance.getClientVersion();
      final packageInfo = await PackageInfo.fromPlatform();
      final current = packageInfo.version;
      final remote = data['version'] as String?;
      if (remote != null && _versionNewer(remote, current) && mounted) {
        setState(() {
          _updateInfo = {
            'version': remote,
            'changelog': data['changelog'] ?? '',
            'sizeBytes': data['size_bytes'] ?? 0,
          };
        });
      }
    } catch (_) {}
  }

  int get _totalReleases =>
      _apps.fold(0, (sum, a) => sum + ((a['release_count'] as int?) ?? 0));

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Store'),
        actions: [
          IconButton(
            icon: const Icon(Icons.settings),
            onPressed: () => context.push('/settings'),
          ),
        ],
      ),
      body: _loading
          ? const Center(
              child: CircularProgressIndicator(color: AppColors.primary),
            )
          : Column(
              children: [
                const Divider(height: 1),
                if (_error != null) ErrorBanner(message: _error!),
                if (_updateInfo != null && !_updateDismissed)
                  UpdateBanner(
                    version: _updateInfo!['version'] as String,
                    onTap: () {
                      final packageInfo = PackageInfo.fromPlatform();
                      packageInfo.then((info) {
                        context.push('/update', extra: {
                          'currentVersion': info.version,
                          'newVersion': _updateInfo!['version'],
                          'changelog': _updateInfo!['changelog'],
                          'sizeBytes': _updateInfo!['sizeBytes'],
                        });
                      });
                    },
                    onDismiss: () =>
                        setState(() => _updateDismissed = true),
                  ),
                Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 16,
                    vertical: 10,
                  ),
                  child: Row(
                    children: [
                      const Icon(Icons.widgets_outlined,
                          size: 14, color: AppColors.textTertiary),
                      const SizedBox(width: 6),
                      Text(
                        '${_apps.length} app${_apps.length != 1 ? 's' : ''}',
                        style: const TextStyle(
                          fontSize: 12,
                          color: AppColors.textTertiary,
                        ),
                      ),
                      const SizedBox(width: 16),
                      const Icon(Icons.local_offer_outlined,
                          size: 14, color: AppColors.textTertiary),
                      const SizedBox(width: 6),
                      Text(
                        '$_totalReleases release${_totalReleases != 1 ? 's' : ''}',
                        style: const TextStyle(
                          fontSize: 12,
                          color: AppColors.textTertiary,
                        ),
                      ),
                    ],
                  ),
                ),
                const Divider(height: 1),
                Expanded(
                  child: RefreshIndicator(
                    onRefresh: _fetchApps,
                    color: AppColors.primary,
                    child: _apps.isEmpty
                        ? ListView(
                            children: [
                              Padding(
                                padding: const EdgeInsets.only(top: 80),
                                child: Column(
                                  children: const [
                                    Icon(Icons.storefront_outlined,
                                        size: 48,
                                        color: AppColors.textTertiary),
                                    SizedBox(height: 12),
                                    Text(
                                      'Aucune application',
                                      style: TextStyle(
                                        color: AppColors.textSecondary,
                                        fontSize: 16,
                                      ),
                                    ),
                                    SizedBox(height: 4),
                                    Text(
                                      'Les publications sont g\u00e9r\u00e9es via MCP.',
                                      style: TextStyle(
                                        color: AppColors.textTertiary,
                                        fontSize: 13,
                                      ),
                                    ),
                                  ],
                                ),
                              ),
                            ],
                          )
                        : ListView.builder(
                            padding: EdgeInsets.zero,
                            itemCount: _apps.length,
                            itemBuilder: (context, index) {
                              final app = _apps[index];
                              return AppCard(
                                app: app,
                                onTap: () {
                                  final slug = app['slug'] as String;
                                  final name = app['name'] as String?;
                                  context.push(
                                    '/app/$slug${name != null ? '?name=${Uri.encodeComponent(name)}' : ''}',
                                  );
                                },
                              );
                            },
                          ),
                  ),
                ),
              ],
            ),
    );
  }
}
