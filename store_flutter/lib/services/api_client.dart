import 'package:dio/dio.dart';
import 'secure_storage.dart' as storage;

const _defaultServerUrl = 'https://proxy.mynetwk.biz';

class ApiClient {
  ApiClient._();
  static final instance = ApiClient._();

  late Dio _dio;
  String _baseUrl = _defaultServerUrl;

  Future<void> init() async {
    final stored = await storage.getServerUrl();
    _baseUrl = (stored != null && stored.isNotEmpty) ? stored : _defaultServerUrl;
    _dio = Dio(BaseOptions(
      baseUrl: '$_baseUrl/api',
      headers: {'Content-Type': 'application/json'},
    ));
  }

  Future<void> setBaseUrl(String url) async {
    final clean = url.replaceAll(RegExp(r'/+$'), '');
    await storage.setServerUrl(clean);
    _baseUrl = clean;
    _dio = Dio(BaseOptions(
      baseUrl: '$clean/api',
      headers: {'Content-Type': 'application/json'},
    ));
  }

  String get baseUrl => _baseUrl;

  Future<dynamic> getStoreApps() async {
    final response = await _dio.get('/store/apps');
    return response.data;
  }

  Future<dynamic> getStoreApp(String slug) async {
    final response = await _dio.get('/store/apps/$slug');
    return response.data;
  }

  Future<dynamic> checkUpdates(Map<String, String> installed) async {
    final param = installed.entries
        .map((e) => '${e.key}:${e.value}')
        .join(',');
    final response = await _dio.get('/store/updates?installed=$param');
    return response.data;
  }

  Future<dynamic> getClientVersion() async {
    final response = await _dio.get('/store/client/version');
    return response.data;
  }

  String getDownloadUrl(String slug, String version) {
    return '$_baseUrl/api/store/releases/$slug/$version/download';
  }

  String getClientApkUrl() {
    return '$_baseUrl/api/store/client/apk';
  }

  Future<Response> downloadFile({
    required String url,
    required String savePath,
    void Function(int, int)? onProgress,
    CancelToken? cancelToken,
  }) async {
    return await _dio.download(
      url,
      savePath,
      onReceiveProgress: onProgress,
      cancelToken: cancelToken,
    );
  }
}
