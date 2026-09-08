!macro NSIS_HOOK_PREUNINSTALL
  IfSilent +2
  MessageBox MB_OK|MB_ICONINFORMATION "CyberKindred 卸载程序只删除应用文件。用户数据会保留；如需删除，请先在应用设置中执行“全部重置”。"
!macroend
