#include <libavutil/avutil.h>
#include <libavutil/error.h>
#include <libavutil/opt.h>
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavfilter/avfilter.h>
#include <libavfilter/buffersink.h>
#include <libswscale/swscale.h>

enum MotionFfmpegConstants {
	MOTION_AVERROR_EOF = AVERROR_EOF,
};