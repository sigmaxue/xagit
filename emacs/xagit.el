;;; xagit.el --- Open xagit in a floating window from Emacs  -*- lexical-binding: t; -*-

;; Author: sigmaxue
;; Maintainer: sigmaxue
;; URL: https://github.com/sigmaxue/xagit
;; Version: 1.0.0
;; Package-Requires: ((emacs "25.1"))
;; Keywords: vc, git, tools, terminals
;;
;; xagit is a Magit-style full-screen git TUI written in Rust.  This package
;; runs it inside a real terminal emulator (term.el) and shows that buffer in
;; a large, dedicated, near-fullscreen window (a "floating" window), then
;; restores your window layout when xagit exits.  On GUI frames, if the
;; optional `posframe' package is installed, a true floating child frame is
;; used instead.
;;
;; No external dependency is required: term.el ships with Emacs.

;;; Commentary:
;;
;; Usage:
;;   M-x xagit
;; Suggested binding:
;;   (global-set-key (kbd "C-c x g") #'xagit)
;;
;; Inside xagit the keys are sent verbatim, including C-c C-c (commit/save),
;; C-c C-k (cancel), C-x C-c (quit) and C-g (abort).  This package sets the
;; term escape key to C-^ so those keys are passed through to the TUI; use
;; C-^ to reach Emacs' term commands (e.g. C-^ b to switch buffers).
;; When xagit exits the window closes and the previous layout is restored.
;;
;; Configuration (set before/after loading, via setq or Customize):
;;   xagit-executable   path to the binary ("xagit" = look in `exec-path')
;;   xagit-directory    directory to open: nil = `default-directory',
;;                      a string, or a function returning a directory
;;                      (xagit itself walks up to the git toplevel)
;;   xagit-width        fraction of frame width   (default 0.9)
;;   xagit-height       fraction of frame height  (default 0.85)
;;   xagit-posframe     use posframe when available (default t)
;;   xagit-buffer-name  terminal buffer name      (default "*xagit*")
;;   xagit-term-escape-key  keymap escape key     (default ?\C-^)

;;; Code:

(require 'term)
(require 'cl-lib)

;; posframe is an optional dependency (used only when installed).
(declare-function posframe-show "posframe")
(declare-function posframe-delete "posframe")
(declare-function posframe--get-posframe "posframe")
(declare-function posframe-poshandler-frame-center "posframe")

(defgroup xagit nil
  "Open xagit (a Magit-style git TUI) in a floating window."
  :group 'vc
  :prefix "xagit-")

(defcustom xagit-executable "xagit"
  "The xagit program.
Either an absolute path or a name looked up in `exec-path'.  When unset
and no `xagit' is in PATH, a build at ../target/{release,debug}/xagit
relative to this file is tried."
  :type 'string
  :group 'xagit)

(defcustom xagit-directory nil
  "Directory in which xagit is started.
NIL means `default-directory' of the current buffer.  A string is used as
given (expanded); a function is called with no arguments and must return a
directory.  xagit walks up to the git toplevel itself."
  :type '(choice (const :tag "Current buffer's directory" nil)
                 string
                 function)
  :group 'xagit)

(defcustom xagit-width 0.9
  "Fraction of the frame width used by the xagit window (0.0 - 1.0)."
  :type 'float
  :group 'xagit)

(defcustom xagit-height 0.85
  "Fraction of the frame height used by the xagit window (0.0 - 1.0)."
  :type 'float
  :group 'xagit)

(defcustom xagit-posframe t
  "If non-nil, use `posframe' for a real floating frame when available.
Falls back to a dedicated large window otherwise (and always in a terminal)."
  :type 'boolean
  :group 'xagit)

(defcustom xagit-buffer-name "*xagit*"
  "Name of the terminal buffer running xagit."
  :type 'string
  :group 'xagit)

(defcustom xagit-term-escape-key ?\C-^
  "Escape key for the xagit term buffer.
xagit uses C-c and C-x heavily, so the escape key is moved away from C-c
to this key; C-c / C-x are then passed straight through to the TUI."
  :type 'character
  :group 'xagit)

(defvar-local xagit--window-configuration nil
  "Window layout to restore when the xagit buffer is closed.")
(defvar-local xagit--frame nil
  "Floating frame used for the xagit buffer, if any.")
(defvar-local xagit--posframe-buffer nil
  "Non-nil when this buffer is shown via posframe.")
(defvar xagit--raw-map nil
  "Keymap used by xagit terminal buffers (C-c / C-x passed through).")

;;; Internal helpers

(defun xagit--find-executable ()
  "Return an absolute path to the xagit executable, or nil."
  (cond
   ((file-name-absolute-p xagit-executable)
    (and (file-executable-p xagit-executable) xagit-executable))
   ((executable-find xagit-executable))
   (t
    ;; <repo>/emacs/xagit.el -> <repo>/target/{release,debug}/xagit
    (catch 'found
      (when load-file-name
        (let ((root (file-name-directory
                     (directory-file-name
                      (file-name-directory load-file-name)))))
          (dolist (rel '("target/release/xagit" "target/debug/xagit"))
            (let ((cand (expand-file-name rel root)))
              (when (file-executable-p cand)
                (throw 'found cand))))))
      nil))))

(defun xagit--repo-directory ()
  "Resolve the directory xagit should be started in."
  (let ((d (cond
            ((functionp xagit-directory) (funcall xagit-directory))
            ((stringp xagit-directory) (expand-file-name xagit-directory))
            (t default-directory))))
    (file-name-as-directory (expand-file-name d))))

(defun xagit--make-raw-map ()
  "Build a term-raw-map copy that passes C-c / C-x straight to xagit."
  (let ((map (copy-keymap term-raw-map)))
    ;; Move the term escape prefix off C-c and onto xagit-term-escape-key.
    (define-key map (char-to-string ?\C-c) #'term-send-raw)
    (define-key map (kbd "C-x") #'term-send-raw)
    (define-key map (kbd "C-g") #'term-send-raw)
    (let ((esc (vector xagit-term-escape-key)))
      (define-key map esc (lookup-key term-raw-map (kbd "C-c")))
      ;; pressing the escape key twice sends a raw escape char
      (define-key map (vconcat esc esc) #'term-send-raw))
    map))

(defun xagit--raw-map ()
  "Return (memoized) xagit terminal keymap."
  (or xagit--raw-map (setq xagit--raw-map (xagit--make-raw-map))))

;;; Display

(defun xagit--geometry ()
  "Return (WIDTH HEIGHT) for the xagit window."
  (list (max 20 (min (- (frame-width) 2)
                     (floor (* (frame-width) xagit-width))))
        (max 8 (min (- (frame-height) 2)
                    (floor (* (frame-height) xagit-height))))))

(defun xagit--posframe-available-p ()
  (and xagit-posframe (display-graphic-p) (require 'posframe nil t)
       (fboundp 'posframe-show)))

(defun xagit--display (buffer)
  "Show BUFFER in a floating window/frame; return a cleanup thunk."
  (cl-destructuring-bind (w h) (xagit--geometry)
    (cond
     ;; GUI with posframe: a true floating child frame.
     ((xagit--posframe-available-p)
      (with-current-buffer buffer (setq xagit--posframe-buffer t))
      (posframe-show buffer
                     :width w :height h
                     :poshandler #'posframe-poshandler-frame-center
                     :border-width 1
                     :border-color (face-foreground 'font-lock-keyword-face nil "gray")
                     :internal-border-width 1
                     :internal-border-color (face-foreground 'font-lock-keyword-face nil "gray")
                     :background-color (face-background 'default nil "black")
                     :respect-header-line nil
                     :min-width w :min-height h)
      (let ((frame (posframe--get-posframe buffer)))
        (when (frame-live-p frame) (select-frame-set-input-focus frame)))
      (lambda () (posframe-delete buffer)))
     ;; Fallback: an exclusive large window, layout restored on close.
     (t
      (let ((config (current-window-configuration)))
        (with-current-buffer buffer
          (setq xagit--window-configuration config))
        (delete-other-windows)
        (switch-to-buffer buffer)
        (lambda ()
          (when (and xagit--window-configuration
                     (window-configuration-p xagit--window-configuration))
            (set-window-configuration xagit--window-configuration))))))))

;;; Cleanup

(defun xagit--cleanup (buffer)
  "Close windows/frames showing BUFFER and kill its terminal buffer."
  (when (buffer-live-p buffer)
    (with-current-buffer buffer
      (when (functionp (buffer-local-value 'xagit--cleanup-fn buffer))
        (funcall (buffer-local-value 'xagit--cleanup-fn buffer))))
    ;; delete any normal window showing it
    (dolist (win (get-buffer-window-list buffer t t))
      (when (window-live-p win) (delete-window win)))
    ;; kill the buffer (and its process)
    (when (buffer-live-p buffer)
      (let ((proc (get-buffer-process buffer)))
        (when (and proc (process-live-p proc)) (delete-process proc)))
      (kill-buffer buffer))))

(defvar-local xagit--cleanup-fn nil
  "Thunk that closes the floating display of this xagit buffer.")

(defun xagit--sentinel (buffer)
  "Return a process sentinel for BUFFER that cleans up when xagit exits."
  (lambda (_proc _event)
    (when (buffer-live-p buffer)
      (let ((proc (get-buffer-process buffer)))
        ;; called on exit / signal
        (when (or (null proc) (not (process-live-p proc)))
          (run-at-time 0 nil #'xagit--cleanup buffer))))))

;;; Live process reuse

(defun xagit--live-buffer ()
  "Return the running xagit buffer, if any."
  (let ((buf (get-buffer xagit-buffer-name)))
    (and buf (get-buffer-process buf)
         (process-live-p (get-buffer-process buf))
         buf)))

;;;###autoload
(defun xagit ()
  "Open xagit in a floating window.
If xagit is already running, focus its window.  The repository is taken
from `xagit-directory' (default: the current buffer's directory); xagit
itself walks up to the git toplevel.  Quit xagit with C-x C-c."
  (interactive)
  (let ((live (xagit--live-buffer)))
    (if live
        (xagit--focus live)
      (let ((exe (xagit--find-executable)))
        (unless exe
          (user-error "xagit executable not found; set `xagit-executable' to its full path"))
        (let* ((dir (xagit--repo-directory))
               (default-directory dir)
               (buffer (make-term xagit-buffer-name exe nil dir)))
          (with-current-buffer buffer
            (term-char-mode)
            ;; C-c / C-x / C-g pass straight through to xagit
            (use-local-map (xagit--raw-map))
            (setq xagit--cleanup-fn (xagit--display buffer))
            (setq mode-line-format
                  '("  " mode-line-buffer-identification
                    "   xagit — C-x C-c quit, C-^ <key> for Emacs term commands"))
            (let ((proc (get-buffer-process buffer)))
              (when proc
                (set-process-sentinel proc (xagit--sentinel buffer))
                (set-process-query-on-exit-flag proc nil)))))))))

(defun xagit--focus (buffer)
  "Focus the window/frame showing BUFFER."
  (if (buffer-local-value 'xagit--posframe-buffer buffer)
      (let ((frame (and (require 'posframe nil t)
                        (posframe--get-posframe buffer))))
        (when (frame-live-p frame) (select-frame-set-input-focus frame)))
    (pop-to-buffer buffer)))

;;;###autoload
(defun xagit-quit ()
  "Close the running xagit window and buffer."
  (interactive)
  (let ((buf (get-buffer xagit-buffer-name)))
    (if buf (xagit--cleanup buf) (message "No xagit window is open"))))

(provide 'xagit)

;;; xagit.el ends here
