import { Glitch } from '@/components/Glitch';
import { Button } from '@/components/ui/button';
import { invoke } from '@tauri-apps/api/core';

export const LogIn = () => {
  return (
    <div className="flex flex-col h-screen justify-center align-center mt-[-36px]">
      <div className="flex justify-center">
        <Button
          onClick={() => {
            invoke('login');
          }}
          className={`bg-[#9146FF] text-center align-center cursor-pointer`}
        >
          <Glitch />
          Login with Twitch
        </Button>
      </div>
    </div>
  );
};
