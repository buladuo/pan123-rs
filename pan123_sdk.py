import shlex

import requests
import hashlib
import os
import json
import logging
import uuid
import time
import random
import urllib.parse
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import List, Dict, Optional, Tuple

import argparse
import sys
import qrcode
from tqdm import tqdm

# 配置日志输出
logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(levelname)s - %(message)s')

TOKEN_FILE = "123pan_token.json"

class Pan123Client:
    def __init__(self, token: Optional[str] = None):
        """
        初始化 123pan 客户端
        """
        self.session = requests.Session()
        self.base_url = "https://www.123pan.com/api"
        self.ucenter_url = "https://login.123pan.com"

        self.loginuuid = hashlib.sha256(str(uuid.uuid4()).encode()).hexdigest()

        self.headers = {
            "Accept": "application/json, text/plain, */*",
            "Accept-Encoding": "gzip, deflate, br, zstd",
            "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.8,en-GB;q=0.7,en-US;q=0.6",
            "App-Version": "3",
            "Connection": "keep-alive",
            "LoginUuid": self.loginuuid,
            "Origin": "https://www.123pan.com",
            "Referer": "https://www.123pan.com/",
            "Sec-Fetch-Dest": "empty",
            "Sec-Fetch-Mode": "cors",
            "Sec-Fetch-Site": "same-origin",
            "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36 Edg/146.0.0.0",
            "platform": "web",
            "sec-ch-ua": '"Chromium";v="146", "Not-A.Brand";v="24", "Microsoft Edge";v="146"',
            "sec-ch-ua-mobile": "?0",
            "sec-ch-ua-platform": '"Windows"'
        }

        self.session.headers.update(self.headers)
        self._init_domains()

        if not token:
            token = self._load_local_token()
        if token:
            self._apply_token(token)

    def _init_domains(self):
        logging.info("正在获取动态API域名路由...")
        dydomain_url = "https://login.123pan.com/api/dydomain"
        try:
            res = self.session.get(dydomain_url).json()
            if res.get("code") == 0:
                data = res.get("data", {})
                domains = data.get("domains", ["www.123pan.com"])
                if domains:
                    self.base_url = f"https://{domains[0]}/api"
                ucenter_domain = data.get("ucenterDomain", "login.123pan.com")
                self.ucenter_url = f"https://{ucenter_domain}"
        except Exception as e:
            logging.error(f"请求动态域名接口异常: {e}")

    def _get_domain(self) -> str:
        """内部辅助方法，快速获取当前主站域名"""
        return self.base_url.split('/api')[0] if '/api' in self.base_url else "https://www.123pan.com"

    def _apply_token(self, token: str):
        self.session.headers["authorization"] = f"Bearer {token}"
        self.session.cookies.set("sso-token", token, domain=".123pan.com")

    def _save_local_token(self, token: str):
        try:
            with open(TOKEN_FILE, "w", encoding="utf-8") as f:
                json.dump({"token": token, "update_time": time.strftime("%Y-%m-%d %H:%M:%S")}, f)
        except Exception as e:
            logging.error(f"保存本地 Token 失败: {e}")

    def _load_local_token(self) -> Optional[str]:
        if os.path.exists(TOKEN_FILE):
            try:
                with open(TOKEN_FILE, "r", encoding="utf-8") as f:
                    return json.load(f).get("token")
            except Exception:
                pass
        return None

    def check_token_valid(self) -> bool:
        if "authorization" not in self.session.headers:
            return False
        try:
            list_url = f"{self._get_domain()}/b/api/file/list/new"
            params = {
                "driveId": "0", "limit": 1, "next": "0", "orderBy": "update_time",
                "orderDirection": "desc", "parentFileId": "0", "trashed": "false", "Page": 1, "operateType": "1"
            }
            params.update(self._get_dynamic_params())
            res = self.session.get(list_url, params=params).json()
            return res.get("code") == 0
        except Exception:
            return False

    def login_by_qrcode(self) -> bool:
        logging.info("正在请求生成登录二维码...")
        generate_url = f"{self.ucenter_url}/api/user/qr-code/generate"
        try:
            res = self.session.get(generate_url).json()
            if res.get("code") != 0:
                return False
            uni_id = res.get("data", {}).get("uniID")
            scan_url = f"https://www.123pan.com/wx-app-login.html?env=production&uniID={uni_id}&source=123pan&type=login"
            logging.info("请扫描下方二维码：")
            self._print_qr_code(scan_url)
            return self._poll_qr_result(uni_id)
        except Exception:
            return False

    def _print_qr_code(self, url: str):
        try:

            qr = qrcode.QRCode(version=1, box_size=1, border=1)
            qr.add_data(url)
            qr.make(fit=True)
            qr.print_ascii(invert=True)
            logging.info(f"直接访问: {url}")
        except ImportError:
            logging.info(f"访问: {url}")

    def _poll_qr_result(self, uni_id: str) -> bool:
        poll_url = f"{self.ucenter_url}/api/user/qr-code/result"
        params = {"uniID": uni_id}
        max_retries, last_status = 120, -1

        for _ in range(max_retries):
            try:
                res = self.session.get(poll_url, params=params).json()
                code, data, message = res.get("code"), res.get("data", {}), res.get("message", "")

                if code == 200 and "token" in data:
                    token = data.get("token")
                    self._apply_token(token)
                    self._save_local_token(token)
                    logging.info("扫码登录成功 (123pan App)！")
                    return True
                elif code == 0:
                    status = data.get("loginStatus")
                    if status != last_status:
                        if status == 1: logging.info("已扫码，请确认...")
                        elif status == 4:
                            wx_res = self.session.post(f"{self.ucenter_url}/api/user/qr-code/wx_code", json={"uniID": uni_id}).json()
                            if wx_res.get("code") == 0:
                                wx_code = wx_res.get("data", {}).get("wxCode")
                                login_res = self.session.post(f"{self.ucenter_url}/api/user/sign_in", json={"from": "web", "wechat_code": wx_code, "type": 4}).json()
                                if login_res.get("code") in [0, 200]:
                                    token = login_res.get("data", {}).get("token") or self.session.cookies.get("sso-token")
                                    if token:
                                        self._apply_token(token)
                                        self._save_local_token(token)
                                        logging.info("微信扫码登录成功！")
                                        return True
                            return False
                        last_status = status
                elif code in [401, 403, 404] or "失效" in message:
                    return False
            except Exception: pass
            time.sleep(1.5)
        return False

    def _get_dynamic_params(self) -> Dict[str, str]:
        key = random.randint(0, 2**31-1)
        value = f"{int(time.time())}-{random.randint(0, 10**7)}-{random.randint(0, 10**10)}"
        return {str(key): value}

    # ================= 基础查询逻辑 =================

    def get_user_info(self) -> Dict:
        url = f"{self._get_domain()}/b/api/restful/goapi/v1/user/report/info"
        try:
            return self.session.get(url, params=self._get_dynamic_params()).json().get("data", {})
        except Exception:
            return {}

    def get_file_info(self, file_ids: List[int]) -> List[Dict]:
        url = f"{self._get_domain()}/b/api/file/info"
        payload = {"fileIdList": [{"fileId": fid} for fid in file_ids]}
        try:
            res = self.session.post(url, params=self._get_dynamic_params(), json=payload).json()
            return res.get("data", {}).get("infoList", [])
        except Exception:
            return []

    def get_file_list(self, parent_id: str = "0", page: int = 1, limit: int = 100) -> List[Dict]:
        list_url = f"{self._get_domain()}/b/api/file/list/new"
        params = {
            "driveId": "0", "limit": limit, "next": "0", "orderBy": "update_time",
            "orderDirection": "desc", "parentFileId": parent_id, "trashed": "false",
            "Page": page, "operateType": "1"
        }
        params.update(self._get_dynamic_params())
        try:
            res = self.session.get(list_url, params=params).json()
            return res.get("data", {}).get("InfoList", [])
        except Exception:
            return []

    # ================= 进阶：文件管理 (重命名/移动/复制/删除) =================

    def rename_file(self, file_id: int, new_name: str) -> bool:
        """重命名文件或文件夹"""
        logging.info(f"正在重命名文件 ID:{file_id} -> '{new_name}'")
        url = f"{self._get_domain()}/b/api/file/rename"
        payload = {
            "driveId": 0,
            "fileId": file_id,
            "fileName": new_name,
            "duplicate": 1,
            "RequestSource": None
        }
        try:
            res = self.session.post(url, params=self._get_dynamic_params(), json=payload).json()
            if res.get("code") == 0:
                return True
            logging.error(f"重命名失败: {res.get('message')}")
        except Exception as e:
            logging.error(f"重命名异常: {e}")
        return False

    def move_files(self, file_ids: List[int], target_parent_id: str) -> bool:
        """移动文件或文件夹到指定目录"""
        logging.info(f"正在移动 {len(file_ids)} 个项目至目录 ID:{target_parent_id}")
        url = f"{self._get_domain()}/b/api/file/mod_pid"
        payload = {
            "fileIdList": [{"FileId": fid} for fid in file_ids], # 注意抓包中是大写 F
            "parentFileId": int(target_parent_id),
            "event": "fileMove",
            "operatePlace": "bottom",
            "RequestSource": None
        }
        try:
            res = self.session.post(url, params=self._get_dynamic_params(), json=payload).json()
            if res.get("code") == 0:
                return True
            logging.error(f"移动失败: {res.get('message')}")
        except Exception as e:
            logging.error(f"移动异常: {e}")
        return False

    def delete_files(self, file_ids: List[int]) -> bool:
        """删除文件（放入回收站）"""
        logging.info(f"准备删除 {len(file_ids)} 个项目...")
        # 删除需要完整的 FileInfo，这里我们使用刚才写的 get_file_info 来获取完整元数据
        full_infos = self.get_file_info(file_ids)
        if not full_infos:
            logging.error("无法获取文件的详细信息，删除终止。")
            return False

        url = f"{self._get_domain()}/b/api/file/trash"
        payload = {
            "driveId": 0,
            "fileTrashInfoList": full_infos, # get_file_info 返回的结构恰好完美匹配 Trash 所需的结构
            "operation": True,
            "event": "intoRecycle",
            "operatePlace": "bottom",
            "RequestSource": None,
            "safeBox": False
        }
        try:
            res = self.session.post(url, params=self._get_dynamic_params(), json=payload).json()
            if res.get("code") == 0:
                logging.info("移入回收站成功！")
                return True
            logging.error(f"删除失败: {res.get('message')}")
        except Exception as e:
            logging.error(f"删除异常: {e}")
        return False

    def copy_files(self, file_ids: List[int], target_parent_id: str) -> bool:
        """复制文件（异步轮询机制）"""
        logging.info(f"正在准备复制 {len(file_ids)} 个项目至目录 ID:{target_parent_id}...")
        # 复制同样需要一些元数据，但格式(小驼峰)与获取的信息(大驼峰)不同，需要进行转换
        full_infos = self.get_file_info(file_ids)
        if not full_infos:
            return False

        file_list_payload = []
        for info in full_infos:
            file_list_payload.append({
                "fileId": info.get("FileId"),
                "size": info.get("Size"),
                "etag": info.get("Etag"),
                "type": info.get("Type"),
                "parentFileId": info.get("ParentFileId"),
                "fileName": info.get("FileName"),
                "driveId": 0
            })

        domain = self._get_domain()
        copy_url = f"{domain}/b/api/restful/goapi/v1/file/copy/async"
        payload = {
            "fileList": file_list_payload,
            "targetFileId": int(target_parent_id)
        }

        try:
            # 1. 提交异步复制任务
            res = self.session.post(copy_url, params=self._get_dynamic_params(), json=payload).json()
            if res.get("code") != 0:
                logging.error(f"提交复制任务失败: {res.get('message')}")
                return False

            task_id = res.get("data", {}).get("taskId")
            if not task_id:
                return False

            logging.info(f"成功提交复制任务，TaskID: {task_id}，正在等待服务器处理...")

            # 2. 轮询任务状态
            task_url = f"{domain}/b/api/restful/goapi/v1/file/copy/task"
            for _ in range(30): # 最大等待 30 秒
                task_params = {"taskId": task_id}
                task_params.update(self._get_dynamic_params())

                task_res = self.session.get(task_url, params=task_params).json()
                if task_res.get("code") == 0:
                    status = task_res.get("data", {}).get("status")
                    if status == 2: # 状态 2 代表成功
                        logging.info("✅ 服务器后台复制完成！")
                        return True
                    # 状态可能是 1(处理中) 或 0(等待中)，继续轮询
                time.sleep(1)

            logging.warning("轮询复制任务状态超时，请前往网盘检查。")
            return False
        except Exception as e:
            logging.error(f"复制过程异常: {e}")
            return False

    # ================= 下载相关逻辑 =================

    def _check_download_traffic(self, file_ids: List[int]) -> bool:
        url = f"{self._get_domain()}/b/api/file/download/traffic/check"
        try:
            res = self.session.post(url, params=self._get_dynamic_params(), json={"fids": file_ids}).json()
            if res.get("code") == 0:
                data = res.get("data", {})
                if data.get("isTrafficExceeded"):
                    logging.error("下载失败：账号流量已超限！")
                    return False
                return True
            else:
                logging.error(f"流量检查失败: {res.get('message')}")
                return False
        except Exception as e:
            logging.error(f"流量检查异常: {e}")
            return False

    def get_download_url(self, file_ids: List[int]) -> Tuple[str, str]:
        if not file_ids:
            return "", ""
        if not self._check_download_traffic(file_ids):
            return "", ""

        domain = self._get_domain()
        is_single_file = False
        file_metadata = {}

        if len(file_ids) == 1:
            infos = self.get_file_info(file_ids)
            if infos and infos[0].get("Type") == 0:
                is_single_file = True
                file_metadata = infos[0]

        try:
            if is_single_file:
                logging.info("正在获取单文件下载链接...")
                url = f"{domain}/b/api/v2/file/download_info"
                payload = {
                    "driveId": 0, "etag": file_metadata.get("Etag", ""),
                    "fileId": file_metadata.get("FileId"), "s3keyFlag": file_metadata.get("S3KeyFlag", ""),
                    "type": file_metadata.get("Type", 0), "fileName": file_metadata.get("FileName", ""),
                    "size": file_metadata.get("Size", 0)
                }
            else:
                logging.info("正在获取批量下载链接...")
                url = f"{domain}/b/api/v2/file/batch_download_info"
                payload = {"fileIdList": [{"fileId": fid} for fid in file_ids]}

            res = self.session.post(url, params=self._get_dynamic_params(), json=payload).json()

            if res.get("code") == 0:
                data = res.get("data", {})
                dispatch_list = data.get("dispatchList", [])
                download_path = data.get("downloadPath", "")

                if dispatch_list and download_path:
                    final_url = dispatch_list[0].get("prefix", "") + download_path
                    parsed_url = urllib.parse.urlparse(final_url)
                    query_params = urllib.parse.parse_qs(parsed_url.query)
                    filename = query_params.get("filename", ["downloaded_file"])[0]
                    filename = urllib.parse.unquote(filename)
                    if not is_single_file:
                        filename = filename+".zip"
                    return final_url, filename
            else:
                logging.error(f"获取下载链接失败: {res.get('message')}")
        except Exception as e:
            logging.error(f"获取下载链接异常: {e}")

        return "", ""

    def download_files(self, file_ids: List[int], save_dir: str = "."):
        logging.info(f"准备下载文件，IDs: {file_ids}")
        download_url, filename = self.get_download_url(file_ids)
        if not download_url:
            return

        save_path = os.path.join(save_dir, filename)
        logging.info(f"获取直链成功！准备保存至: {save_path}")

        try:
            with self.session.get(download_url, stream=True) as r:
                r.raise_for_status()
                total_size = int(r.headers.get('content-length', 0))

                with open(save_path, 'wb') as f, tqdm(
                    total=total_size,
                    unit='B',
                    unit_scale=True,
                    unit_divisor=1024,
                    desc=filename,
                    ncols=100
                ) as pbar:

                    for chunk in r.iter_content(chunk_size=8192):
                        if chunk:
                            f.write(chunk)
                            pbar.update(len(chunk))

            logging.info(f"🎉 下载完成！文件已保存至: {save_path}")

        except Exception as e:
            logging.error(f"下载过程中发生异常: {e}")

    # ================= 上传相关逻辑 =================

    def create_folder(self, folder_name: str, parent_id: str = "0") -> Optional[Dict]:
        logging.info(f"正在创建文件夹: {folder_name} (父目录ID: {parent_id})")
        url = f"{self._get_domain()}/b/api/file/upload_request"

        payload = {
            "driveId": 0, "etag": "", "fileName": folder_name,
            "parentFileId": int(parent_id), "size": 0, "type": 1,
            "duplicate": 1, "NotReuse": True, "RequestSource": None
        }

        try:
            res = self.session.post(url, params=self._get_dynamic_params(), json=payload).json()
            if res.get("code") == 0:
                folder_info = res.get("data", {}).get("Info", {})
                logging.info(f"📁 文件夹 '{folder_name}' 创建成功！ID: {folder_info.get('FileId')}")
                return folder_info
            else:
                logging.error(f"创建文件夹失败: {res.get('message')}")
        except Exception as e:
            logging.error(f"创建文件夹异常: {e}")
        return None

    def _calculate_file_md5(self, file_path: str) -> str:
        hash_md5 = hashlib.md5()
        with open(file_path, "rb") as f:
            for chunk in iter(lambda: f.read(4096 * 1024), b""):
                hash_md5.update(chunk)
        return hash_md5.hexdigest()

    def upload_file(self, file_path: str, parent_id: str = "0", duplicate_mode: int = 1, ask_on_conflict: bool = True) -> Optional[Dict]:
        if not os.path.exists(file_path): return None
        filename, file_size = os.path.basename(file_path), os.path.getsize(file_path)
        logging.info(f"准备上传: '{filename}' ({file_size / 1024 / 1024:.2f} MB)")

        md5_hash = self._calculate_file_md5(file_path)
        domain = self._get_domain()

        # 1. 预检
        req_payload = {"driveId": 0, "etag": md5_hash, "fileName": filename, "parentFileId": int(parent_id), "size": file_size, "type": 0}
        try:
            req_res = self.session.post(f"{domain}/b/api/file/upload_request", params=self._get_dynamic_params(), json=req_payload).json()
            if req_res.get("code") != 0:

                if req_res.get("code") == 5060:  # 文件重名
                    logging.info("检测到同名文件")

                    # === 是否询问用户 ===
                    if ask_on_conflict:
                        while True:
                            user_input = input("文件已存在，选择操作:[0] 取消上传 [1] 保留两者 [2] 完全覆盖 > ").strip()
                            if user_input in ("1", "2"):
                                duplicate = int(user_input)
                                break
                            elif user_input == "0":
                                logging.info("取消上传")
                                return None
                            print("输入无效，请输入 1 或 2")
                    else:
                        duplicate = duplicate_mode
                        logging.info(f"使用默认策略 duplicate={duplicate}")

                    # 写入参数
                    req_payload["duplicate"] = duplicate

                    # 重新请求
                    req_res = self.session.post(
                        f"{domain}/b/api/file/upload_request",
                        params=self._get_dynamic_params(),
                        json=req_payload
                    ).json()

                    if req_res.get("code") != 0:
                        logging.error(f"重试预检失败: {req_res.get('message')}")
                        return None

                else:
                    logging.error(f"预检失败: {req_res.get('message')}")
                    return None
            data = req_res.get("data", {})
            if data.get("Reuse"):
                logging.info(f"⚡ 秒传成功: {filename}")
                return data.get("Info")

            upload_id, bucket, key, storage_node = data.get("UploadId"), data.get("Bucket"), data.get("Key"), data.get("StorageNode")
            temp_file_id, slice_size = data.get("FileId"), int(data.get("SliceSize", 16777216))
            parts_count = max(1, (file_size + slice_size - 1) // slice_size)
            is_multipart = parts_count > 1

            # 2. 分片上传
            with open(file_path, 'rb') as f:
                for part_num in range(1, parts_count + 1):
                    logging.info(f"传输中... [{part_num}/{parts_count}]")

                    # 智能分支：根据是否多切片选择不同的获取 S3 直链的接口
                    if is_multipart:
                        auth_url = f"{domain}/b/api/file/s3_repare_upload_parts_batch"
                    else:
                        auth_url = f"{domain}/b/api/file/s3_upload_object/auth"

                    auth_payload = {
                        "bucket": bucket, "key": key,
                        "partNumberStart": part_num, "partNumberEnd": part_num + 1,
                        "uploadId": upload_id, "StorageNode": storage_node
                    }
                    auth_res = self.session.post(auth_url, params=self._get_dynamic_params(), json=auth_payload).json()

                    put_url = auth_res.get("data", {}).get("presignedUrls", {}).get(str(part_num))
                    if not put_url:
                        logging.error(f"获取分片 S3 预签名链接失败 (is_multipart: {is_multipart})")
                        return None

                    chunk_data = f.read(slice_size)
                    requests.put(put_url, data=chunk_data, headers={"Content-Length": str(len(chunk_data))}, timeout=60).raise_for_status()

            # 3. 合并通知
            complete_payload = {
                "fileId": temp_file_id, "bucket": bucket, "fileSize": file_size,
                "key": key, "isMultipart": is_multipart, "uploadId": upload_id,
                "StorageNode": storage_node
            }
            comp_res = self.session.post(f"{domain}/b/api/file/upload_complete/v2", params=self._get_dynamic_params(), json=complete_payload).json()

            if comp_res.get("code") != 0:
                logging.error(f"上传合并申请失败: {comp_res.get('message')}")
                return None

            if not is_multipart and comp_res.get("data", {}).get("file_info"):
                final_info = comp_res.get("data", {}).get("file_info")
                if final_info.get("FileId"):
                    logging.info(f"✅ 上传成功: {filename}")
                    return final_info

            # 4. 结果验证
            logging.info("等待服务器最终合并...")
            for _ in range(30): # 轮询最多 60 秒
                check_info = self.get_file_info([temp_file_id])
                if check_info and check_info[0].get("Status") == 0:
                    logging.info(f"✅ 通过详情确认上传成功: {filename}")
                    return check_info[0]

                time.sleep(1)
            return None
        except Exception as e:
            logging.error(f"上传异常: {e}")
            return None
    def upload_directory(self, local_dir: str, parent_id: str = "0", max_workers: int = 3):
        if not os.path.isdir(local_dir):
            logging.error(f"目录不存在: {local_dir}")
            return

        base_name = os.path.basename(os.path.normpath(local_dir))
        logging.info(f"开始上传整个目录: {base_name}")

        root_folder_info = self.create_folder(base_name, parent_id)
        if not root_folder_info:
            return

        root_remote_id = str(root_folder_info.get("FileId"))
        dir_mapping = {os.path.abspath(local_dir): root_remote_id}
        upload_tasks = []

        for root, dirs, files in os.walk(local_dir):
            current_remote_parent_id = dir_mapping.get(os.path.abspath(root))
            if not current_remote_parent_id:
                continue

            for d in dirs:
                local_d_path = os.path.abspath(os.path.join(root, d))
                sub_folder_info = self.create_folder(d, current_remote_parent_id)
                if sub_folder_info:
                    dir_mapping[local_d_path] = str(sub_folder_info.get("FileId"))

            for f in files:
                local_f_path = os.path.abspath(os.path.join(root, f))
                upload_tasks.append((local_f_path, current_remote_parent_id))

        total_files = len(upload_tasks)
        logging.info(f"目录结构创建完毕，共有 {total_files} 个文件准备并发上传...")

        success_count = 0
        with ThreadPoolExecutor(max_workers=max_workers) as executor:
            future_to_file = {
                executor.submit(self.upload_file, path, p_id): path
                for path, p_id in upload_tasks
            }
            for future in as_completed(future_to_file):
                try:
                    if future.result():
                        success_count += 1
                except Exception as exc:
                    logging.error(f"文件上传异常: {exc}")

        logging.info(f"🎉 文件夹 [{base_name}] 任务结束！文件成功率: {success_count}/{total_files}")


CWD_FILE = "123pan_cwd.json"

class Pan123Cli:
    def __init__(self):
        self.client = Pan123Client()
        self.parser = self._setup_parser()

    def get_cwd(self) -> dict:
        """获取当前工作目录记录"""
        if os.path.exists(CWD_FILE):
            try:
                with open(CWD_FILE, "r", encoding="utf-8") as f:
                    return json.load(f)
            except Exception:
                pass
        return {"file_id": 0, "path": "/"}

    def set_cwd(self, file_id: int, path: str):
        """保存当前工作目录记录"""
        try:
            with open(CWD_FILE, "w", encoding="utf-8") as f:
                json.dump({"file_id": file_id, "path": path}, f)
        except Exception as e:
            print(f"保存工作目录状态失败: {e}")

    def _setup_parser(self) -> argparse.ArgumentParser:
        """设置命令行参数解析器"""
        parser = argparse.ArgumentParser(
            description="123云盘 命令行客户端 (基于 Pan123 SDK)",
            formatter_class=argparse.RawTextHelpFormatter
        )
        subparsers = parser.add_subparsers(dest="command", title="可用命令", help="输入 '<命令> -h' 查看具体用法")

        # 1. 登录
        subparsers.add_parser("login", help="扫码登录 123云盘")

        # 2. 用户信息
        subparsers.add_parser("info", help="查看当前登录用户信息")

        # 3. 切换和查看目录
        subparsers.add_parser("pwd", help="显示当前工作目录 (CWD)")

        cd_parser = subparsers.add_parser("cd", help="切换当前工作目录")
        cd_parser.add_argument("target", help="目标目录名称、'..' (上级) 或 '/' (根目录)")
        cd_parser.add_argument("--id", action="store_true", help="指定 target 为目录 FileId 而非名称")

        # 4. 文件列表
        ls_parser = subparsers.add_parser("ls", help="列出指定目录下的文件")
        ls_parser.add_argument("-p", "--parent", dest="parent_id", default=None, help="父目录 ID，默认使用当前目录 (CWD)")
        ls_parser.add_argument("-l", "--limit", type=int, default=100, help="最多显示的条目数 (默认 100)")

        # 4.5 树状目录
        tree_parser = subparsers.add_parser("tree", help="以树状图显示目录结构")
        tree_parser.add_argument("-p", "--parent", dest="parent_id", default=None, help="父目录 ID，默认使用当前目录 (CWD)")
        tree_parser.add_argument("-d", "--depth", type=int, default=3, help="遍历深度 (默认 3，过大可能导致请求缓慢)")

        # 5. 上传
        upload_parser = subparsers.add_parser("upload", help="上传文件或目录到网盘")
        upload_parser.add_argument("local_path", help="本地文件或文件夹的路径")
        upload_parser.add_argument("-p", "--parent", dest="parent_id", default=None, help="目标父目录 ID，默认使用当前目录 (CWD)")
        upload_parser.add_argument("-w", "--workers", type=int, default=3, help="并发线程数 (仅对目录上传有效，默认 3)")

        # 6. 下载
        download_parser = subparsers.add_parser("download", help="下载指定的文件或目录")
        download_parser.add_argument("file_ids", nargs="+", type=int, help="要下载的文件/目录 ID (可多个)")
        download_parser.add_argument("-d", "--dir", dest="save_dir", default=".", help="保存的本地目录，默认为当前目录")

        # 7. 新建文件夹
        mkdir_parser = subparsers.add_parser("mkdir", help="新建文件夹")
        mkdir_parser.add_argument("folder_name", help="新文件夹名称")
        mkdir_parser.add_argument("-p", "--parent", dest="parent_id", default=None, help="目标父目录 ID，默认使用当前目录 (CWD)")

        # 8. 重命名
        rename_parser = subparsers.add_parser("rename", help="重命名文件或文件夹")
        rename_parser.add_argument("file_id", type=int, help="要重命名的文件/目录 ID")
        rename_parser.add_argument("new_name", help="新的名称")

        # 8. 移动
        mv_parser = subparsers.add_parser("mv", help="移动文件或文件夹")
        mv_parser.add_argument("target_parent_id", help="目标父目录 ID")
        mv_parser.add_argument("file_ids", nargs="+", type=int, help="要移动的文件/目录 ID (可多个)")

        # 9. 复制
        cp_parser = subparsers.add_parser("cp", help="复制文件或文件夹")
        cp_parser.add_argument("target_parent_id", help="目标父目录 ID")
        cp_parser.add_argument("file_ids", nargs="+", type=int, help="要复制的文件/目录 ID (可多个)")

        # 10. 删除
        rm_parser = subparsers.add_parser("rm", help="删除文件或目录(放入回收站)")
        rm_parser.add_argument("file_ids", nargs="+", type=int, help="要删除的文件/目录 ID (可多个)")

        return parser

    def _ensure_auth(self) -> bool:
        """确保用户已登录"""
        if not self.client.check_token_valid():
            print("❌ 当前未登录或登录已过期，请先扫码登录！")
            success = self.client.login_by_qrcode()
            if not success:
                print("❌ 登录失败。")
                return False
        return True

    def execute_command(self, args):
        """执行具体的命令逻辑"""
        if not args.command:
            return

        # 登录命令不需要前置鉴权检查
        if args.command == "login":
            if self.client.check_token_valid():
                print("✅ 您已经处于登录状态，无需重复登录。")
            else:
                self.client.login_by_qrcode()
            return

        # 执行其他命令前，确保已鉴权
        if not self._ensure_auth():
            return

        # 根据命令分发执行
        if args.command == "info":
            info = self.client.get_user_info()
            print(json.dumps(info, ensure_ascii=False, indent=2))

        elif args.command == "pwd":
            cwd = self.get_cwd()
            print(f"当前目录: {cwd['path']} (ID: {cwd['file_id']})")

        elif args.command == "cd":
            cwd = self.get_cwd()
            current_id = cwd["file_id"]
            current_path = cwd["path"]
            target = args.target

            if target == "/":
                self.set_cwd(0, "/")
                print("✅ 已切换至根目录: /")
            elif target == "..":
                if str(current_id) == "0":
                    print("⚠️ 已经在根目录。")
                else:
                    info = self.client.get_file_info([int(current_id)])
                    if info:
                        parent_id = info[0].get("ParentFileId", 0)
                        new_path = "/".join(current_path.rstrip("/").split("/")[:-1])
                        if not new_path:
                            new_path = "/"
                        self.set_cwd(parent_id, new_path)
                        print(f"✅ 已切换至上级目录: {new_path} (ID: {parent_id})")
                    else:
                        print("❌ 无法获取当前目录信息。")
            elif args.id:
                # 按 ID 切换
                info = self.client.get_file_info([int(target)])
                if info and info[0].get("Type") != 0:
                    folder_name = info[0].get("FileName", target)
                    new_path = f"{current_path.rstrip('/')}/{folder_name}"
                    self.set_cwd(int(target), new_path)
                    print(f"✅ 已切换至目录 ID: {target} ({new_path})")
                else:
                    print(f"❌ 找不到 ID 为 {target} 的目录，或者它不是一个目录。")
            else:
                # 按名称在当前目录下搜索
                files = self.client.get_file_list(parent_id=str(current_id), limit=100)
                target_file = None
                for f in files:
                    if f.get("FileName") == target and f.get("Type") != 0:
                        target_file = f
                        break

                if target_file:
                    target_id = target_file.get("FileId")
                    new_path = f"{current_path.rstrip('/')}/{target}"
                    self.set_cwd(target_id, new_path)
                    print(f"✅ 已切换至目录: {new_path} (ID: {target_id})")
                else:
                    print(f"❌ 当前目录下找不到名为 '{target}' 的文件夹。")

        elif args.command == "ls":
            parent_id = args.parent_id if args.parent_id is not None else self.get_cwd()["file_id"]
            files = self.client.get_file_list(parent_id=str(parent_id), limit=args.limit)
            if not files:
                print("当前目录为空或获取失败。")
            else:
                print(f"{'类型':<5} | {'文件 ID':<12} | {'大小':<10} | {'名称'}")
                print("-" * 65)
                for item in files:
                    item_type = "DIR " if item.get("Type") != 0 else "FILE"
                    size_mb = f"{item.get('Size', 0) / (1024*1024):.2f} MB" if item.get("Type") == 0 else "-"
                    print(f"{item_type:<5} | {item.get('FileId'):<12} | {size_mb:<10} | {item.get('FileName')}")
                print("-" * 65)
                print(f"总计: {len(files)} 项")

        elif args.command == "tree":
            parent_id = args.parent_id if args.parent_id is not None else self.get_cwd()["file_id"]
            cwd_path = self.get_cwd()["path"] if args.parent_id is None else f"ID:{parent_id}"
            print(f"📁 {cwd_path}")
            self._print_tree(str(parent_id), max_depth=args.depth)

        elif args.command == "upload":
            parent_id = args.parent_id if args.parent_id is not None else self.get_cwd()["file_id"]
            path = args.local_path
            if not os.path.exists(path):
                print(f"❌ 路径不存在: {path}")
                return
            if os.path.isdir(path):
                self.client.upload_directory(local_dir=path, parent_id=str(parent_id), max_workers=args.workers)
            else:
                self.client.upload_file(file_path=path, parent_id=str(parent_id))

        elif args.command == "download":
            self.client.download_files(file_ids=args.file_ids, save_dir=args.save_dir)

        elif args.command == "mkdir":
            parent_id = args.parent_id if args.parent_id is not None else self.get_cwd()["file_id"]
            self.client.create_folder(folder_name=args.folder_name, parent_id=str(parent_id))

        elif args.command == "rename":
            success = self.client.rename_file(file_id=args.file_id, new_name=args.new_name)
            if success:
                print(f"✅ 文件 {args.file_id} 已重命名为 '{args.new_name}'")

        elif args.command == "mv":
            success = self.client.move_files(file_ids=args.file_ids, target_parent_id=args.target_parent_id)
            if success:
                print(f"✅ 成功将 {len(args.file_ids)} 个项目移动至目录 {args.target_parent_id}")

        elif args.command == "cp":
            success = self.client.copy_files(file_ids=args.file_ids, target_parent_id=args.target_parent_id)
            if success:
                print(f"✅ 成功复制项目！")

        elif args.command == "rm":
            success = self.client.delete_files(file_ids=args.file_ids)
            if success:
                print(f"✅ 成功将 {len(args.file_ids)} 个项目移入回收站。")

    def _print_tree(self, parent_id: str, prefix: str = "", current_depth: int = 1, max_depth: int = 3):
        """递归打印目录树"""
        if current_depth > max_depth:
            return

        files = self.client.get_file_list(parent_id=parent_id, limit=1000)
        if not files:
            return

        count = len(files)
        for i, item in enumerate(files):
            is_last = (i == count - 1)
            connector = "└── " if is_last else "├── "

            is_dir = item.get("Type") != 0
            item_type = "📁" if is_dir else "📄"
            name = item.get("FileName", "Unknown")

            print(f"{prefix}{connector}{item_type} {name} (ID: {item.get('FileId')})")

            if is_dir and current_depth < max_depth:
                extension = "    " if is_last else "│   "
                self._print_tree(str(item.get("FileId")), prefix + extension, current_depth + 1, max_depth)

    def run_interactive(self):
        """运行交互式命令行 (Shell 模式)"""
        print("========================================")
        print("    欢迎进入 123云盘 交互式命令行！")
        print("  输入 'help' 或 '?' 查看可用命令。")
        print("  输入 'exit' 或 'quit' 退出。")
        print("========================================")

        # 初始进入前先验证登录状态
        if not self._ensure_auth():
            return

        while True:
            cwd = self.get_cwd()
            try:
                # 动态显示当前所在路径
                cmd_line = input(f"123pan ({cwd['path']}) > ").strip()
            except (KeyboardInterrupt, EOFError):
                print("\n退出交互模式。")
                break

            if not cmd_line:
                continue

            if cmd_line.lower() in ['exit', 'quit']:
                break

            if cmd_line.lower() in ['help', '?']:
                self.parser.print_help()
                continue

            try:
                # [新增]: 将 Windows 路径的反斜杠替换为双反斜杠，防止被 shlex 当作转义字符吞掉
                safe_cmd_line = cmd_line.replace('\\', '\\\\')

                # 使用 shlex 解析命令字符串，支持带引号的包含空格的文件名
                args_list = shlex.split(safe_cmd_line)
                args = self.parser.parse_args(args_list)
                self.execute_command(args)
            except SystemExit:
                # 拦截 argparse 在遇到错误参数或 -h 时调用的 sys.exit()
                # 保证交互环境不会因为打错命令而退出
                pass
            except Exception as e:
                print(f"执行出错: {e}")

    def run(self):
        """主入口，根据启动参数分发运行模式"""
        if len(sys.argv) > 1:
            # 单行命令模式: 例如 `python pan123_cli.py ls`
            args = self.parser.parse_args()
            self.execute_command(args)
        else:
            # 交互式 Shell 模式: 直接运行 `python pan123_cli.py`
            self.run_interactive()

if __name__ == "__main__":
    cli = Pan123Cli()
    cli.run()
