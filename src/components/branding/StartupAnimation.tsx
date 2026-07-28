import { motion } from "framer-motion";
import { useEffect } from "react";

interface StartupAnimationProps {
  onComplete: () => void;
}

export function StartupAnimation({ onComplete }: StartupAnimationProps) {
  useEffect(() => {
    const timer = setTimeout(onComplete, 2000);
    return () => clearTimeout(timer);
  }, [onComplete]);

  return (
    <motion.div
      className="flex h-full w-full flex-col items-center justify-center"
      onClick={onComplete}
      exit={{ opacity: 0, transition: { duration: 0.4 } }}
    >
      <motion.h1
        className="bg-gradient-to-r from-indigo-300 via-white to-indigo-300 bg-clip-text text-5xl font-bold tracking-tight text-transparent"
        style={{ backgroundSize: "200% 100%" }}
        initial={{ opacity: 0, scale: 0.85, letterSpacing: "0.3em" }}
        animate={{
          opacity: 1,
          scale: 1,
          letterSpacing: "0.02em",
          backgroundPosition: ["0% 50%", "100% 50%"],
        }}
        transition={{
          opacity: { duration: 0.6, ease: "easeOut" },
          scale: { duration: 0.6, ease: "easeOut" },
          letterSpacing: { duration: 0.8, ease: "easeOut" },
          backgroundPosition: {
            duration: 1.6,
            ease: "linear",
            repeat: Infinity,
            repeatType: "mirror",
          },
        }}
      >
        Ace
      </motion.h1>
      <motion.p
        className="mt-2 text-xs text-white/40"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ delay: 0.8, duration: 0.5 }}
      >
        your desk, every model
      </motion.p>
    </motion.div>
  );
}
