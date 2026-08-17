package dev.oligami.fastexplorer;

import com.google.firebase.messaging.FirebaseMessagingService;
import com.google.firebase.messaging.RemoteMessage;

public final class FastExplorerMessagingService extends FirebaseMessagingService {
    @Override
    public void onNewToken(String token) {
        FastExplorerPush.saveToken(this, token);
    }

    @Override
    public void onMessageReceived(RemoteMessage message) {
        RemoteMessage.Notification notification = message.getNotification();
        String title = notification != null && notification.getTitle() != null
                ? notification.getTitle()
                : "FastExplorer device transfer";
        String detail = notification != null && notification.getBody() != null
                ? notification.getBody()
                : "A file or clipboard item is ready to receive";
        FastExplorerPush.showNotification(this, title, detail);
    }
}
