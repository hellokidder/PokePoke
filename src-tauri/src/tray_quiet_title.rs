//! macOS 托盘静谧模式倒计时:蓝色富文本 title(`MM:SS`)。

use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSColor, NSForegroundColorAttributeName, NSStatusItem};
use objc2_foundation::{NSAttributedString, NSDictionary, NSString};

/// 在托盘按钮上显示蓝色倒计时文本。
pub fn set_blue_title(status_item: &NSStatusItem, title: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        crate::log_warn!("tray", "quiet mode title skipped: not on main thread");
        return;
    };
    let Some(button) = status_item.button(mtm) else {
        return;
    };

    let ns_text = NSString::from_str(title);
    let color = NSColor::systemBlueColor();
    let attr_str = unsafe {
        let keys = [NSForegroundColorAttributeName];
        let values: [&AnyObject; 1] = [&color];
        let attrs = NSDictionary::from_slices(&keys, &values);
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &ns_text,
            Some(&attrs),
        )
    };
    button.setAttributedTitle(&attr_str);
}

/// 清除托盘按钮上的倒计时 title。
pub fn clear(status_item: &NSStatusItem) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(button) = status_item.button(mtm) else {
        return;
    };
    button.setTitle(&NSString::from_str(""));
}
