import { Broadcaster as BroadcasterType } from '@/views/List';
import { Avatar, AvatarFallback, AvatarImage } from './ui/avatar';
import { truncate } from '@/lib/utils';
import { invoke } from '@tauri-apps/api/core';

export const Broadcaster = (broadcaster: BroadcasterType) => {
  const openStream = (broadcaster_name: string) => {
    invoke('open_broadcaster_url', { broadcasterName: broadcaster_name });
  };
  return (
    <div
      id="broadcaster"
      className="flex flex-row hover:cursor-pointer mt-2 mb-3"
      onClick={() => openStream(broadcaster.broadcaster_name)}
      title={broadcaster.is_live ? broadcaster.title : undefined}
    >
      <div className="flex hover:cursor-pointer">
        <Avatar className={`${!broadcaster.is_live && `grayscale`}`}>
          <AvatarImage src={broadcaster.profile_picture} />
          <AvatarFallback></AvatarFallback>
        </Avatar>

        <div className="flex flex-col ml-1 text-sm">
          <p title={broadcaster.broadcaster_name} className="font-bold leading-[15.4px]">
            {broadcaster.broadcaster_name}
          </p>
          <p title={broadcaster.category} className="leading-[19.6px] dark:text-[#adadb8] text-[#53535f]">
            {broadcaster.is_live ? truncate(broadcaster.category) : 'Offline'}
          </p>
        </div>
      </div>
    </div>
  );
};
